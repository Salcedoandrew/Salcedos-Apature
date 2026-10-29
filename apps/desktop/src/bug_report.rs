//! Help › Report a Bug…: what the dialog sends, as a prefilled GitHub
//! "new issue" link. No token or API key is involved: the link opens in the
//! browser and the tester submits it with their own GitHub account.
//!
//! System info is limited to the app version, build and commit, macOS
//! version, CPU architecture, the shortcut layout and the focused photo's
//! file type (its extension only). Never a path, a file name or anything
//! from the photo's metadata.

use std::path::Path;

use crate::prefs::ShortcutLayout;

/// Where bug reports go. The one place the repository is named.
pub const REPO_URL: &str = "https://github.com/Salcedoandrew/salcedos-apature";

/// Longest link we hand to the browser. GitHub (and some browsers) refuse
/// much longer ones, so a longer report is trimmed to fit.
pub const MAX_URL_LEN: usize = 7000;

/// GitHub's limit for an issue title, in characters.
const MAX_TITLE_CHARS: usize = 256;

/// What the tester typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BugReport {
    pub title: String,
    pub happened: String,
    pub steps: String,
    pub expected: String,
}

impl BugReport {
    pub fn can_submit(&self) -> bool {
        !self.title.trim().is_empty()
    }
}

/// The system details a report may include (shown read-only before sending).
#[derive(Clone, Debug, PartialEq)]
pub struct SystemInfo {
    pub version: String,
    pub build: String,
    pub commit: Option<String>,
    pub macos: Option<String>,
    pub arch: String,
    pub layout: ShortcutLayout,
    /// Extension of the focused photo, upper-case (e.g. `NEF`).
    pub file_type: Option<String>,
}

impl SystemInfo {
    /// Gather the details for this run. `focused` is only used for its extension.
    pub fn collect(layout: ShortcutLayout, focused: Option<&Path>) -> SystemInfo {
        SystemInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build: option_env!("APATURE_BUILD").unwrap_or("dev").to_string(),
            commit: option_env!("APATURE_GIT_COMMIT").map(str::to_string),
            macos: macos_version(),
            arch: std::env::consts::ARCH.to_string(),
            layout,
            file_type: focused.and_then(file_type),
        }
    }

    /// Label and value, in the order shown and sent.
    pub fn lines(&self) -> Vec<(&'static str, String)> {
        let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
        vec![
            (
                "Apature",
                format!("{} (build {})", self.version, self.build),
            ),
            ("Commit", or_unknown(&self.commit)),
            ("macOS", or_unknown(&self.macos)),
            ("CPU", self.arch.clone()),
            (
                "Shortcut layout",
                match self.layout {
                    ShortcutLayout::PhotoMechanic => "Photo Mechanic".into(),
                    ShortcutLayout::Classic => "Apature classic".into(),
                },
            ),
            (
                "Focused photo type",
                self.file_type.clone().unwrap_or_else(|| "none".into()),
            ),
        ]
    }
}

/// The photo's extension, upper-case, and nothing else: no directory, no
/// stem. Odd extensions (long, or not plain letters and digits) are left out
/// rather than risk sending part of a name.
pub fn file_type(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    (!ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_uppercase())
}

#[cfg(target_os = "macos")]
fn macos_version() -> Option<String> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!v.is_empty()).then_some(v)
}

#[cfg(not(target_os = "macos"))]
fn macos_version() -> Option<String> {
    None
}

fn cap(s: &str, max: Option<usize>) -> String {
    let s = s.trim();
    match max {
        Some(m) if s.chars().count() > m => {
            let cut: String = s.chars().take(m).collect();
            format!("{}… *(trimmed)*", cut.trim_end())
        }
        _ => s.to_string(),
    }
}

/// The issue body in Markdown, each free-text field capped at `max` characters.
fn body_capped(r: &BugReport, info: Option<&SystemInfo>, max: Option<usize>) -> String {
    let mut b = String::new();
    for (head, text) in [
        ("What happened", &r.happened),
        ("Steps to reproduce", &r.steps),
        ("What you expected", &r.expected),
    ] {
        let t = cap(text, max);
        b += &format!(
            "### {head}\n\n{}\n\n",
            if t.is_empty() { "_No response_" } else { &t }
        );
    }
    if let Some(info) = info {
        b += "### System info\n\n";
        for (k, v) in info.lines() {
            b += &format!("- {k}: {v}\n");
        }
        b += "\n";
    }
    b += "---\n_Sent from Apature › Help › Report a Bug…_\n";
    b
}

/// The full, untrimmed issue body.
pub fn body(r: &BugReport, info: Option<&SystemInfo>) -> String {
    body_capped(r, info, None)
}

/// Title and body as plain Markdown, for Copy to Clipboard.
pub fn clipboard_text(r: &BugReport, info: Option<&SystemInfo>) -> String {
    format!("## {}\n\n{}", r.title.trim(), body(r, info))
}

/// Percent-encode for a query value: everything but RFC 3986's unreserved
/// characters, UTF-8 byte by byte.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn url_for(title: &str, body: &str) -> String {
    format!(
        "{REPO_URL}/issues/new?title={}&body={}&labels=bug",
        encode(title),
        encode(body)
    )
}

/// The link Submit opens.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueLink {
    pub url: String,
    /// The body had to be shortened to keep the link under [`MAX_URL_LEN`].
    pub trimmed: bool,
}

/// Build the prefilled "new issue" link. When the whole report does not fit
/// in [`MAX_URL_LEN`], the free-text fields are shortened (longest first,
/// each marked "trimmed") while the headings and system info stay whole.
pub fn issue_link(r: &BugReport, info: Option<&SystemInfo>) -> IssueLink {
    let title = cap(&r.title, Some(MAX_TITLE_CHARS)).replace('\n', " ");
    let url = url_for(&title, &body(r, info));
    if url.len() <= MAX_URL_LEN {
        return IssueLink {
            url,
            trimmed: false,
        };
    }
    let longest = [&r.happened, &r.steps, &r.expected]
        .iter()
        .map(|s| s.trim().chars().count())
        .max()
        .unwrap_or(0);
    let mut max = longest;
    loop {
        max = max * 4 / 5;
        let url = url_for(&title, &body_capped(r, info, Some(max)));
        if url.len() <= MAX_URL_LEN || max == 0 {
            return IssueLink { url, trimmed: true };
        }
    }
}

/// Open a link in the default browser.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> SystemInfo {
        SystemInfo {
            version: "0.1.0".into(),
            build: "42".into(),
            commit: Some("907d638".into()),
            macos: Some("15.1".into()),
            arch: "aarch64".into(),
            layout: ShortcutLayout::PhotoMechanic,
            file_type: file_type(Path::new("/Users/someone/Pictures/Client X/DSC_0001.NEF")),
        }
    }

    fn report() -> BugReport {
        BugReport {
            title: "Crop & rotate: 100% wrong?".into(),
            happened: "Line one\nLine two #3 = bad".into(),
            steps: "1. Open\n2. Press K".into(),
            expected: String::new(),
        }
    }

    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn param<'a>(url: &'a str, name: &str) -> &'a str {
        let q = url.split_once('?').unwrap().1;
        q.split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .unwrap()
    }

    #[test]
    fn encodes_reserved_and_unicode() {
        assert_eq!(
            encode("a b&c=d?e#f/g+h%"),
            "a%20b%26c%3Dd%3Fe%23f%2Fg%2Bh%25"
        );
        assert_eq!(encode("é›"), "%C3%A9%E2%80%BA");
        assert_eq!(encode("Az09-_.~"), "Az09-_.~");
        assert_eq!(encode("x\ny"), "x%0Ay");
    }

    #[test]
    fn link_has_title_body_and_label() {
        let l = issue_link(&report(), Some(&info()));
        assert!(!l.trimmed);
        assert!(l.url.starts_with(&format!("{REPO_URL}/issues/new?title=")));
        assert!(l.url.ends_with("&labels=bug"));
        assert!(l.url.is_ascii());
        assert_eq!(decode(param(&l.url, "title")), "Crop & rotate: 100% wrong?");
        let body = decode(param(&l.url, "body"));
        assert_eq!(body, super::body(&report(), Some(&info())));
        assert!(body.contains("### What happened\n\nLine one\nLine two #3 = bad"));
        assert!(body.contains("### Steps to reproduce\n\n1. Open\n2. Press K"));
        assert!(body.contains("### What you expected\n\n_No response_"));
        assert!(body.contains("- Apature: 0.1.0 (build 42)"));
        assert!(body.contains("- Commit: 907d638"));
        assert!(body.contains("- macOS: 15.1"));
        assert!(body.contains("- CPU: aarch64"));
        assert!(body.contains("- Shortcut layout: Photo Mechanic"));
        assert!(body.contains("- Focused photo type: NEF"));
    }

    #[test]
    fn system_info_can_be_left_out() {
        let b = body(&report(), None);
        assert!(!b.contains("System info"));
        assert!(!b.contains("aarch64"));
    }

    #[test]
    fn never_sends_paths_or_names() {
        assert_eq!(
            file_type(Path::new("/Users/someone/Pictures/x/DSC_0001.nef")).as_deref(),
            Some("NEF")
        );
        assert_eq!(file_type(Path::new("/Users/someone/no-extension")), None);
        // Something that isn't a plain extension is dropped, not sent.
        assert_eq!(file_type(Path::new("/tmp/a.b c d")), None);
        assert_eq!(file_type(Path::new("/tmp/photo.verylongext")), None);
        let mut i = info();
        i.layout = ShortcutLayout::Classic;
        let l = issue_link(&report(), Some(&i));
        let all = decode(&l.url);
        for bad in ["/Users", "someone", "DSC_0001", "Client X", "Pictures"] {
            assert!(!all.contains(bad), "{bad} leaked");
        }
        assert!(all.contains("- Shortcut layout: Apature classic"));
    }

    #[test]
    fn long_reports_are_trimmed_to_fit() {
        let mut r = report();
        r.happened = "ä long story ".repeat(800);
        r.steps = "step ".repeat(50);
        let full = issue_link(
            &BugReport {
                happened: "short".into(),
                ..r.clone()
            },
            Some(&info()),
        );
        assert!(!full.trimmed);
        let l = issue_link(&r, Some(&info()));
        assert!(l.trimmed);
        assert!(l.url.len() <= MAX_URL_LEN, "{}", l.url.len());
        let body = decode(param(&l.url, "body"));
        assert!(body.contains("*(trimmed)*"));
        // Headings, short fields and the system info survive.
        assert!(body.contains("### What happened"));
        assert!(body.contains("### Steps to reproduce\n\nstep step"));
        assert!(body.contains("- Focused photo type: NEF"));
        assert!(body.ends_with("Report a Bug…_\n"));
        // The clipboard copy keeps everything.
        assert!(clipboard_text(&r, Some(&info())).contains(&r.happened.trim().to_string()));
    }

    #[test]
    fn long_titles_are_capped_and_single_line() {
        let r = BugReport {
            title: format!("{}\nsecond line", "t".repeat(400)),
            ..report()
        };
        let l = issue_link(&r, None);
        let t = decode(param(&l.url, "title"));
        assert!(!t.contains('\n'));
        assert!(t.chars().count() < 300);
    }

    #[test]
    fn title_is_required() {
        assert!(!BugReport::default().can_submit());
        assert!(!BugReport {
            title: "   ".into(),
            ..Default::default()
        }
        .can_submit());
        assert!(report().can_submit());
    }
}
