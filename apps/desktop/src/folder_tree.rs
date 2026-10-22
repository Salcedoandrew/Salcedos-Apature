//! The sidebar's folder tree: which subfolders a folder has, read lazily on
//! a background thread the first time a row is expanded (and again each time
//! it is re-opened), so a slow network share or a sleeping disk never holds
//! up the window. Read-only: nothing here creates, renames or deletes.
//!
//! Listed: real folders, case-insensitively sorted. Hidden: names starting
//! with a dot, folders Finder hides (the `UF_HIDDEN` flag, e.g. ~/Library),
//! and bundles or packages (apps, photo libraries, Lightroom previews…).

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender};

/// One subfolder, as the parent's listing saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub name: String,
    pub path: PathBuf,
    /// It has at least one listed subfolder of its own (shows a triangle).
    pub has_children: bool,
}

#[derive(Clone, Debug)]
pub enum Node {
    /// A read is under way; `old` is the previous listing, shown meanwhile.
    Loading {
        old: Option<Vec<Child>>,
    },
    Ready(Vec<Child>),
    Failed(String),
}

type Loaded = (PathBuf, Result<Vec<Child>, String>);

/// A visible row as last drawn, for keyboard movement.
#[derive(Clone, Debug)]
pub struct RowInfo {
    /// Which tree (favorite or Folders root) the row is under.
    pub section: u64,
    pub path: PathBuf,
    pub expandable: bool,
    pub expanded: bool,
    pub parent: Option<usize>,
}

pub struct FolderTree {
    nodes: HashMap<PathBuf, Node>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    expanded: BTreeSet<PathBuf>,
    roots: Vec<PathBuf>,
    root_ids: Vec<RootId>,
    /// Keyboard cursor: (section, path).
    pub cursor: Option<(u64, PathBuf)>,
    /// A single click waiting out the double-click interval.
    pub pending_click: Option<(PathBuf, Instant)>,
    /// The folder the tree last revealed (expanded to and scrolled to).
    pub revealed_for: Option<PathBuf>,
    pub reveal_since: Option<Instant>,
    /// Rows in draw order from the last frame.
    pub rows: Vec<RowInfo>,
    /// The keyboard moved the cursor: scroll its row into view.
    pub scroll_cursor: bool,
}

impl FolderTree {
    pub fn new(expanded: impl IntoIterator<Item = PathBuf>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        FolderTree {
            nodes: HashMap::new(),
            tx,
            rx,
            expanded: expanded.into_iter().collect(),
            roots: Vec::new(),
            root_ids: Vec::new(),
            cursor: None,
            pending_click: None,
            revealed_for: None,
            reveal_since: None,
            rows: Vec::new(),
            scroll_cursor: false,
        }
    }

    pub fn is_expanded(&self, p: &Path) -> bool {
        self.expanded.contains(p)
    }

    /// Expanded folders, for prefs.
    pub fn expanded_list(&self) -> Vec<PathBuf> {
        self.expanded.iter().cloned().collect()
    }

    /// Expand `p` and (re)read its subfolders.
    pub fn expand(&mut self, p: &Path, repaint: Option<&egui::Context>) {
        self.expanded.insert(p.to_path_buf());
        self.request(p, repaint);
    }

    pub fn collapse(&mut self, p: &Path) {
        self.expanded.remove(p);
    }

    /// Expand without reading now (the read happens when the row is drawn).
    pub fn mark_expanded(&mut self, p: &Path) -> bool {
        self.expanded.insert(p.to_path_buf())
    }

    pub fn node(&self, p: &Path) -> Option<&Node> {
        self.nodes.get(p)
    }

    /// The listing to show for `p`: the fresh one, or the previous one
    /// while a re-read is under way.
    pub fn children(&self, p: &Path) -> Option<&[Child]> {
        match self.nodes.get(p)? {
            Node::Ready(c) => Some(c),
            Node::Loading { old: Some(c) } => Some(c),
            _ => None,
        }
    }

    pub fn is_loading(&self, p: &Path) -> bool {
        matches!(self.nodes.get(p), Some(Node::Loading { .. }))
    }

    /// Start a background read of `p` unless one is already running.
    pub fn request(&mut self, p: &Path, repaint: Option<&egui::Context>) {
        if self.is_loading(p) {
            return;
        }
        let old = self.children(p).map(|c| c.to_vec());
        self.nodes.insert(p.to_path_buf(), Node::Loading { old });
        let tx = self.tx.clone();
        let dir = p.to_path_buf();
        let ctx = repaint.cloned();
        let spawned = std::thread::Builder::new()
            .name("folder-tree".into())
            .spawn(move || {
                let r = list_subfolders(&dir).map_err(|e| e.to_string());
                let _ = tx.send((dir, r));
                if let Some(c) = ctx {
                    c.request_repaint();
                }
            });
        if let Err(e) = spawned {
            self.nodes
                .insert(p.to_path_buf(), Node::Failed(e.to_string()));
        }
    }

    /// Take finished reads; true when anything changed.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((p, r)) = self.rx.try_recv() {
            // A root that went away meanwhile (an unplugged card) is dropped.
            if !self.roots.is_empty() && !self.roots.iter().any(|r| p.starts_with(r)) {
                self.nodes.remove(&p);
                continue;
            }
            self.nodes.insert(
                p,
                match r {
                    Ok(c) => Node::Ready(c),
                    Err(e) => Node::Failed(e),
                },
            );
            any = true;
        }
        any
    }

    /// The tree's top-level folders (favorites, Home, volumes). When a root
    /// appears or disappears, everything cached under it is forgotten so a
    /// re-mounted card is read afresh. True when the roots changed.
    #[cfg(test)]
    pub fn sync_roots(&mut self, roots: &[PathBuf]) -> bool {
        let ids: Vec<RootId> = roots.iter().map(|r| (r.clone(), 0)).collect();
        self.sync_root_ids(&ids)
    }

    /// [`Self::sync_roots`] with device ids: a root whose path stays but
    /// whose device changes (a drive mounted over its mount point) counts as
    /// gone and back, so its stale listing is dropped and read again.
    pub fn sync_root_ids(&mut self, roots: &[RootId]) -> bool {
        if self.root_ids == roots {
            return false;
        }
        let changed: Vec<PathBuf> = self
            .root_ids
            .iter()
            .filter(|r| !roots.contains(r))
            .chain(roots.iter().filter(|r| !self.root_ids.contains(r)))
            .map(|(p, _)| p.clone())
            .collect();
        self.nodes
            .retain(|p, _| !changed.iter().any(|r| p.starts_with(r)));
        let paths: Vec<PathBuf> = roots.iter().map(|(p, _)| p.clone()).collect();
        if let Some((_, c)) = &self.cursor {
            if !paths.iter().any(|r| c.starts_with(r)) {
                self.cursor = None;
            }
        }
        self.roots = paths;
        self.root_ids = roots.to_vec();
        true
    }

    /// Expand every folder between a root and `target` (not `target`
    /// itself), so its row is visible. True when anything new was expanded.
    pub fn expand_to(&mut self, target: &Path) -> bool {
        let mut any = false;
        let roots = self.roots.clone();
        for root in roots.iter().filter(|r| target.starts_with(r)) {
            for a in ancestors_between(root, target) {
                any |= self.mark_expanded(&a);
            }
        }
        any
    }
}

/// Is a favorite's drive mounted? A favorite under `/Volumes/<name>` is
/// connected only while that volume is mounted (listed in `mounted`, or its
/// mount point exists); anything else (Home, the startup disk) always is.
pub fn favorite_connected(fav: &Path, mounted: &[PathBuf]) -> bool {
    connected_in(fav, mounted, Path::new("/Volumes"))
}

fn connected_in(fav: &Path, mounted: &[PathBuf], volumes_dir: &Path) -> bool {
    let Ok(rest) = fav.strip_prefix(volumes_dir) else {
        return true;
    };
    let Some(name) = rest.components().next() else {
        return true;
    };
    let vol = volumes_dir.join(name);
    mounted.iter().any(|m| m == &vol) || vol.is_dir()
}

/// A root and the device it lives on (0 when unknown). The same path on a
/// different device is a different root: an empty mount point that was read
/// before the drive mounted over it.
pub type RootId = (PathBuf, u64);

/// The device a folder is on, 0 when it can't be read.
pub fn device_of(p: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(p).map(|m| m.dev()).unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        0
    }
}

/// The tree's roots: connected favorites, then Home, then each mounted
/// volume (with its device id). A favorite on an unmounted drive is left
/// out, and a favorite on a drive carries that drive's device id, so when
/// the drive mounts its root is new and everything cached under it is read
/// afresh, even though the same path was a root already.
pub fn tree_roots(favorites: &[PathBuf], home: Option<&Path>, mounted: &[RootId]) -> Vec<RootId> {
    roots_in(favorites, home, mounted, Path::new("/Volumes"))
}

fn roots_in(
    favorites: &[PathBuf],
    home: Option<&Path>,
    mounted: &[RootId],
    volumes_dir: &Path,
) -> Vec<RootId> {
    let paths: Vec<PathBuf> = mounted.iter().map(|(p, _)| p.clone()).collect();
    let mut roots: Vec<RootId> = favorites
        .iter()
        .filter(|f| connected_in(f, &paths, volumes_dir))
        .map(|f| {
            let dev = mounted
                .iter()
                .find(|(v, _)| f.starts_with(v))
                .map(|(_, d)| *d)
                .unwrap_or(0);
            (f.clone(), dev)
        })
        .collect();
    roots.extend(home.map(|h| (h.to_path_buf(), 0)));
    roots.extend(mounted.iter().cloned());
    roots
}

/// The dim line under an expanded row when it has no subfolder rows to
/// show: still loading, empty, unreadable, or its drive isn't mounted.
pub fn note_for(node: Option<&Node>, connected: bool) -> Option<String> {
    if !connected {
        return Some("Not connected".into());
    }
    match node {
        None | Some(Node::Loading { old: None }) => Some("Loading…".into()),
        Some(Node::Loading { old: Some(c) }) if c.is_empty() => Some("Loading…".into()),
        Some(Node::Ready(c)) if c.is_empty() => Some("No subfolders".into()),
        Some(Node::Failed(e)) => {
            let why = e.split(" (os error").next().unwrap_or("").trim();
            Some(if why.is_empty() {
                "Can't be read".into()
            } else {
                format!("Can't be read · {why}")
            })
        }
        _ => None,
    }
}

/// `root`, then each folder below it, down to `target`'s parent.
/// Empty when `target` is `root` or not inside it.
pub fn ancestors_between(root: &Path, target: &Path) -> Vec<PathBuf> {
    let Ok(rest) = target.strip_prefix(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut at = root.to_path_buf();
    let parts: Vec<_> = rest.components().collect();
    if parts.is_empty() {
        return out;
    }
    out.push(at.clone());
    for c in &parts[..parts.len() - 1] {
        at.push(c);
        out.push(at.clone());
    }
    out
}

/// Subfolders of `dir` the tree shows, sorted case-insensitively.
pub fn list_subfolders(dir: &Path) -> std::io::Result<Vec<Child>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if !is_listed(&e, &name, &path) {
            continue;
        }
        let has_children = has_listed_subfolder(&path);
        out.push(Child {
            name,
            path,
            has_children,
        });
    }
    sort_children(&mut out);
    Ok(out)
}

pub fn sort_children(c: &mut [Child]) {
    c.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn has_listed_subfolder(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        is_listed(&e, &name, &e.path())
    })
}

fn is_listed(e: &fs::DirEntry, name: &str, path: &Path) -> bool {
    if name.starts_with('.') {
        return false;
    }
    // Plain files are ruled out from the directory entry alone (no stat).
    if let Ok(ft) = e.file_type() {
        if ft.is_file() {
            return false;
        }
    }
    let Ok(md) = fs::metadata(path) else {
        return false;
    };
    md.is_dir() && !is_finder_hidden(path) && !is_package(path, name)
}

/// Finder's hidden flag (`chflags hidden`), on the entry or what it links to.
pub fn is_finder_hidden(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        let flag = |m: std::io::Result<fs::Metadata>| {
            m.map(|m| m.st_flags() & UF_HIDDEN != 0).unwrap_or(false)
        };
        flag(fs::symlink_metadata(path)) || flag(fs::metadata(path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

/// Folder extensions macOS treats as a single document or app.
const PACKAGE_EXTS: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "xpc",
    "prefpane",
    "saver",
    "qlgenerator",
    "mdimporter",
    "pkg",
    "mpkg",
    "photoslibrary",
    "photolibrary",
    "migratedphotolibrary",
    "aplibrary",
    "lrdata",
    "lrlibrary",
    "cocatalog",
    "cosessiondb",
    "fcpbundle",
    "imovielibrary",
    "tvlibrary",
    "theater",
    "musiclibrary",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "scptd",
    "wdgt",
    "xcodeproj",
    "xcworkspace",
    "xcarchive",
    "playground",
    "dsym",
    "docarchive",
    "band",
    "logicx",
    "sparsebundle",
    "photosmigrator",
];

/// A bundle or package: a known package extension, or a folder laid out
/// like one (`Contents/Info.plist` or `Contents/PkgInfo`).
pub fn is_package(path: &Path, name: &str) -> bool {
    if let Some((_, ext)) = name.rsplit_once('.') {
        let ext = ext.to_ascii_lowercase();
        if PACKAGE_EXTS.contains(&ext.as_str()) {
            return true;
        }
    }
    let c = path.join("Contents");
    c.join("Info.plist").is_file() || c.join("PkgInfo").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "apature-tree-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn names(c: &[Child]) -> Vec<&str> {
        c.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_sorted_case_insensitively() {
        let d = scratch("sort");
        for n in ["beta", "Alpha", "gamma", "Delta", "alpha2"] {
            fs::create_dir(d.join(n)).unwrap();
        }
        fs::write(d.join("photo.jpg"), b"x").unwrap();
        fs::write(d.join("aaa.txt"), b"x").unwrap();
        let c = list_subfolders(&d).unwrap();
        assert_eq!(names(&c), ["Alpha", "alpha2", "beta", "Delta", "gamma"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn hides_dot_folders_and_packages() {
        let d = scratch("hide");
        for n in [
            ".git",
            ".Trashes",
            "Photos Library.photoslibrary",
            "Apature.APP",
            "Catalog Previews.lrdata",
            "Shoot",
        ] {
            fs::create_dir(d.join(n)).unwrap();
        }
        // A package without a telltale extension.
        fs::create_dir_all(d.join("Odd Thing/Contents")).unwrap();
        fs::write(d.join("Odd Thing/Contents/Info.plist"), b"<plist/>").unwrap();
        // A folder with a dot in its name is still a folder.
        fs::create_dir(d.join("2026.10.22 Storm")).unwrap();
        let c = list_subfolders(&d).unwrap();
        assert_eq!(names(&c), ["2026.10.22 Storm", "Shoot"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn hides_finder_hidden_folders() {
        let d = scratch("flag");
        fs::create_dir(d.join("Library")).unwrap();
        fs::create_dir(d.join("Pictures")).unwrap();
        let ok = std::process::Command::new("chflags")
            .arg("hidden")
            .arg(d.join("Library"))
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert!(is_finder_hidden(&d.join("Library")));
        assert!(!is_finder_hidden(&d.join("Pictures")));
        let c = list_subfolders(&d).unwrap();
        assert_eq!(names(&c), ["Pictures"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn knows_which_children_have_subfolders() {
        let d = scratch("kids");
        fs::create_dir_all(d.join("Assignments/Storm/SELECTS")).unwrap();
        fs::create_dir_all(d.join("Empty")).unwrap();
        fs::create_dir_all(d.join("OnlyHidden/.cache")).unwrap();
        fs::create_dir_all(d.join("OnlyPackage/X.app")).unwrap();
        fs::write(d.join("Empty/a.jpg"), b"x").unwrap();
        let c = list_subfolders(&d).unwrap();
        let has: Vec<(&str, bool)> = c
            .iter()
            .map(|c| (c.name.as_str(), c.has_children))
            .collect();
        assert_eq!(
            has,
            [
                ("Assignments", true),
                ("Empty", false),
                ("OnlyHidden", false),
                ("OnlyPackage", false)
            ]
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn follows_folder_symlinks_but_not_file_links() {
        let d = scratch("link");
        fs::create_dir(d.join("Real")).unwrap();
        fs::write(d.join("file.jpg"), b"x").unwrap();
        std::os::unix::fs::symlink(d.join("Real"), d.join("Linked")).unwrap();
        std::os::unix::fs::symlink(d.join("file.jpg"), d.join("filelink")).unwrap();
        std::os::unix::fs::symlink(d.join("nowhere"), d.join("dangling")).unwrap();
        let c = list_subfolders(&d).unwrap();
        assert_eq!(names(&c), ["Linked", "Real"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unreadable_folder_is_an_error_not_a_panic() {
        let d = scratch("gone");
        assert!(list_subfolders(&d.join("missing")).is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    fn wait(tree: &mut FolderTree, p: &Path) {
        let t0 = Instant::now();
        while tree.is_loading(p) {
            tree.poll();
            assert!(t0.elapsed() < Duration::from_secs(5), "load timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn loads_lazily_in_the_background() {
        let d = scratch("lazy");
        fs::create_dir_all(d.join("B/inner")).unwrap();
        fs::create_dir_all(d.join("a")).unwrap();
        let mut tree = FolderTree::new([]);
        tree.sync_roots(std::slice::from_ref(&d));
        assert!(tree.node(&d).is_none(), "nothing is read before expanding");
        tree.expand(&d, None);
        assert!(tree.is_expanded(&d));
        assert!(tree.is_loading(&d));
        wait(&mut tree, &d);
        let c = tree.children(&d).unwrap();
        assert_eq!(names(c), ["a", "B"]);
        assert!(!c[0].has_children && c[1].has_children);
        assert!(
            tree.node(&d.join("B")).is_none(),
            "children are not read ahead"
        );

        // Re-expanding re-reads, keeping the old list on screen meanwhile.
        fs::create_dir(d.join("C")).unwrap();
        tree.collapse(&d);
        tree.expand(&d, None);
        assert_eq!(names(tree.children(&d).unwrap()), ["a", "B"]);
        wait(&mut tree, &d);
        assert_eq!(names(tree.children(&d).unwrap()), ["a", "B", "C"]);

        // An unreadable folder fails quietly.
        let gone = d.join("gone");
        tree.expand(&gone, None);
        wait(&mut tree, &gone);
        assert!(matches!(tree.node(&gone), Some(Node::Failed(_))));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn roots_changing_forgets_what_was_under_them() {
        let a = scratch("rootA");
        let b = scratch("rootB");
        fs::create_dir(a.join("x")).unwrap();
        fs::create_dir(b.join("y")).unwrap();
        let mut tree = FolderTree::new([]);
        assert!(tree.sync_roots(&[a.clone(), b.clone()]));
        assert!(!tree.sync_roots(&[a.clone(), b.clone()]));
        tree.expand(&a, None);
        tree.expand(&b, None);
        wait(&mut tree, &a);
        wait(&mut tree, &b);
        // b unmounts: its cache goes, a's stays, b stays expanded for next time.
        assert!(tree.sync_roots(std::slice::from_ref(&a)));
        assert!(tree.node(&a).is_some());
        assert!(tree.node(&b).is_none());
        assert!(tree.is_expanded(&b));
        fs::remove_dir_all(&a).unwrap();
        fs::remove_dir_all(&b).unwrap();
    }

    #[test]
    fn volume_root_children_load_once_the_drive_mounts() {
        // "Work Drive" is a favorite and, once mounted, a Folders volume too;
        // its expanded state comes back from prefs before it is mounted.
        let volumes = scratch("vol");
        let home = scratch("volhome");
        let vol = volumes.join("Work Drive");
        let favs = vec![vol.clone()];
        let mut tree = FolderTree::new([vol.clone()]);
        // Not mounted: the favorite is not a root and is never read.
        let roots = roots_in(&favs, Some(&home), &[], &volumes);
        assert_eq!(roots, [(home.clone(), 0)]);
        tree.sync_root_ids(&roots);
        assert!(!connected_in(&vol, &[], &volumes));
        assert_eq!(
            note_for(tree.node(&vol), false).as_deref(),
            Some("Not connected")
        );
        // The mount point shows up empty first and is read (and listed as a
        // volume on the startup disk's device)...
        fs::create_dir_all(&vol).unwrap();
        let roots = roots_in(&favs, Some(&home), &[(vol.clone(), 1)], &volumes);
        assert!(tree.sync_root_ids(&roots));
        tree.request(&vol, None);
        wait(&mut tree, &vol);
        assert_eq!(
            note_for(tree.node(&vol), true).as_deref(),
            Some("No subfolders")
        );
        // ...then the drive mounts over it: same path, another device. The
        // old tree kept the empty listing for good (the path was a root
        // already); now it is forgotten and the drive's folders load.
        fs::create_dir_all(vol.join("2026/Great America")).unwrap();
        fs::create_dir_all(vol.join("EXPORTS")).unwrap();
        let roots = roots_in(&favs, Some(&home), &[(vol.clone(), 2)], &volumes);
        assert_eq!(
            roots[0],
            (vol.clone(), 2),
            "the favorite takes the drive's id"
        );
        assert!(tree.sync_root_ids(&roots));
        assert!(tree.node(&vol).is_none(), "stale listing forgotten");
        assert!(tree.is_expanded(&vol));
        tree.request(&vol, None);
        assert_eq!(note_for(tree.node(&vol), true).as_deref(), Some("Loading…"));
        wait(&mut tree, &vol);
        assert_eq!(names(tree.children(&vol).unwrap()), ["2026", "EXPORTS"]);
        assert_eq!(note_for(tree.node(&vol), true), None);
        assert!(!tree.sync_root_ids(&roots), "nothing changes while mounted");
        fs::remove_dir_all(&volumes).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn notes_for_loading_empty_and_unreadable_folders() {
        assert_eq!(note_for(None, true).as_deref(), Some("Loading…"));
        assert_eq!(
            note_for(Some(&Node::Loading { old: Some(vec![]) }), true).as_deref(),
            Some("Loading…")
        );
        assert_eq!(
            note_for(Some(&Node::Ready(vec![])), true).as_deref(),
            Some("No subfolders")
        );
        assert_eq!(
            note_for(
                Some(&Node::Failed("Operation not permitted (os error 1)".into())),
                true
            )
            .as_deref(),
            Some("Can't be read · Operation not permitted")
        );
        let kid = Child {
            name: "2026".into(),
            path: PathBuf::from("/Volumes/Work Drive/2026"),
            has_children: true,
        };
        assert_eq!(note_for(Some(&Node::Ready(vec![kid])), true), None);
    }

    #[test]
    fn favorites_on_unmounted_drives_are_not_connected() {
        let gone = Path::new("/Volumes/Apature Test No Such Drive/2026");
        assert!(!favorite_connected(gone, &[]));
        assert!(favorite_connected(
            gone,
            &[PathBuf::from("/Volumes/Apature Test No Such Drive")]
        ));
        assert!(favorite_connected(
            Path::new("/Users/someone/Pictures"),
            &[]
        ));
    }

    #[test]
    fn expands_the_way_down_to_a_folder() {
        let root = PathBuf::from("/Users/me");
        let target = PathBuf::from("/Users/me/Pictures/2026/Storm");
        assert_eq!(
            ancestors_between(&root, &target),
            [
                PathBuf::from("/Users/me"),
                PathBuf::from("/Users/me/Pictures"),
                PathBuf::from("/Users/me/Pictures/2026"),
            ]
        );
        assert!(ancestors_between(&root, &root).is_empty());
        assert!(ancestors_between(&root, Path::new("/Volumes/CARD")).is_empty());
        let mut tree = FolderTree::new([PathBuf::from("/kept")]);
        tree.sync_roots(&[root.clone(), PathBuf::from("/Volumes/CARD")]);
        assert!(tree.expand_to(&target));
        assert!(!tree.expand_to(&target));
        assert!(tree.is_expanded(Path::new("/Users/me/Pictures/2026")));
        assert!(!tree.is_expanded(&target));
        assert_eq!(tree.expanded_list().len(), 4);
    }
}
