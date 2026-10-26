# Decision log

Format: ADR-nnn, date, status. Status is one of proposed, accepted,
superseded, or **divergence** (a place where what was built differs from the
brief in `docs/BRIEF.md`).

## ADR-000 (2026-10-13, divergence): the whole app was scaffolded before Phase 0

The brief says: "Do not scaffold the entire desktop application yet"; Phase 0
research and a Phase 1 preview benchmark come first. The first session built
the workspace, engine crates, CLI and an egui window in one pass, before the
brief existed. Rather than throw that away, the code is reclassified: the
engine crates and `apature-cli bench` **are** the Phase 1 feasibility experiment, and
`apps/desktop` is a throwaway rendering probe, not the product UI. The design
process in `docs/design/` decides what the real UI is; nothing in
`apps/desktop/src/app.rs` is treated as settled.

## ADR-001 (2026-10-13, accepted): egui/eframe for the first prototype, not yet for the product

Alternatives: Slint (GPL/royalty-free/commercial triple licence; declarative;
better native text fields), Rust core + SwiftUI/AppKit shell (best native
feel and VoiceOver; two languages; Windows needs a second shell), Tauri
(what Apature did; web renderer; 493-crate graph).

Chosen for the prototype because it keeps everything in one language, draws a
10 000-cell virtual grid with trivial code, and has AccessKit integration.
Known costs, all confirmed in this session by reading egui 0.33's API and
the prototype: no native menu bar beyond winit's default app menu; text input
is egui's own, not NSTextView (no system spell-check, dictation, or emoji
picker); VoiceOver only sees widgets that egui registers, so painter-drawn
cells are invisible; theme follows the system but our panels hard-code
colours. **The product-UI decision is deferred to
`docs/design/DESIGN_IMPLEMENTATION_PLAN.md`**; a thin AppKit layer for menus
and dialogs is the expected compromise if egui stays.

## ADR-002 (2026-10-13, accepted): own TIFF walker for embedded previews; no LibRaw, no rawler

Alternatives: LibRaw via FFI (LGPL-2.1/CDDL; full decode + previews; C++
build), rawler 0.8 (LGPL-2.1; pure Rust; previews + decode + metadata; CR3
support; API unstable; 10–25 min cold build per Apature's ADR-0002),
kamadak-exif (only tiny thumbnails; no CR3).

Chosen: a 300-line IFD reader plus a preview locator that understands
IFD chains, SubIFDs, the Exif IFD and Nikon MakerNote PreviewIFD. Verified
against exiftool on D850 NEFs: identical three previews, identical byte
counts. Zero copyleft in the graph. Cost: CR3 (ISO-BMFF) and HEIC are not
TIFF and need a second locator; no RAW decode fallback exists when a file
has no usable preview. Revisit when CR3 matters or when a develop-for-export
feature is scoped (rawler is the obvious candidate then, kept behind a
feature flag so the default build stays permissive).

## ADR-003 (2026-10-13, accepted): jpeg-decoder for decoding, with DCT scaling

Alternatives: zune-jpeg via `image` (faster full decodes, no scaled decode),
turbojpeg FFI (fastest, C dependency).

Chosen because `Decoder::scale` gives 1/2, 1/4 and 1/8 decodes, which is
what makes thumbnails cheap. Measured: 8 ms per D850 NEF thumbnail from the
1620 px preview, 25 ms from a 2048 px camera JPEG, single-threaded per image,
8 images in flight. Default features off so it does not spawn its own rayon
threads under ours.

## ADR-004 (2026-10-13, accepted): XMP sidecars are edited in place, never regenerated

Alternatives: parse to a model and re-serialise (loses unknown structure
and formatting; what most libraries do), shell out to exiftool for every
write (correct, 100–300 ms per call, external binary).

Chosen: locate each owned property (attribute or element form, any prefix
bound to the namespace) and replace only that span; insert on the first
`rdf:Description` if absent. Verified by tests that a Lightroom-style
sidecar with crs settings, dc bags, xmpMM history and a custom namespace is
byte-identical outside the four owned properties, and by exiftool
`-validate` on both fresh and edited files. Write path: temp file →
`fsync` → rename, with a parse-back check before the rename and the original
left untouched on failure. Limitation: a sidecar whose first
`rdf:Description` is not the one carrying ratings would get a second copy
of the property; not observed in Lightroom or Photo Mechanic output.

## ADR-005 (2026-10-13, accepted): one sidecar per stem, RAW is primary

`DSC_0001.NEF` + `DSC_0001.JPG` share `DSC_0001.xmp`, which is what Photo
Mechanic and Lightroom do for the RAW. **Known gap:** both of those apps
embed XMP *inside* JPEGs rather than reading a sidecar for a JPEG-only item,
so marks we write for JPEG-only folders are invisible to them until we can
embed (Phase 3). `apature-cli scan` reports conflicts between a sidecar and a
sibling JPEG's embedded XMP so the gap is visible rather than silent. **Revised the same day:** marks
are now written to both the sidecar and every JPEG variant (ADR-009).

## ADR-009 (2026-10-13, accepted): pairs get the sidecar and the embedded packet

Photo Mechanic reads a pair's metadata from the JPEG and ignored our
sidecar; Lightroom reads the sidecar for the RAW and the packet for the
JPEG. Writing both keeps every reader consistent. Cost: the JPEG file
changes (mtime, +~150 bytes with an existing packet, ~2.5 KB without),
which the brief's "originals are never modified" rule tolerates only
because the edit is a byte-exact splice of one metadata segment, atomic,
and read-back checked; image data is never re-encoded. Reads prefer the
sidecar when a RAW exists and report conflicts with the JPEG.

## ADR-006 (2026-10-13, accepted): priority queue keyed by viewport distance; no cancellation

Alternatives: rayon (no priorities, no bounded queue; Apature rejected it
for the same reason), LIFO stack with in-flight cap (Apature's choice).

Chosen: a `BTreeSet<(priority, idx, kind)>` where re-requesting an item
updates its priority instead of appending. Stale requests are not cancelled
but are always behind visible ones and still populate the disk cache.
Threads: `available_parallelism - 1`, clamped to 2..=12. Not yet measured
under fast scrolling of a 1 000-item folder; see `docs/STATUS.md`.

## ADR-007 (2026-10-13, accepted, revised same day): colour-class names default to what this Photo Mechanic uses

Photo Mechanic writes the class *name* into `xmp:Label`. exiftool's tag
table describes class 1 as "Winner", but the owner's PM 2026.6 names the
classes Red, Yellow, Green, Blue, Purple, Cyan, Brown, Trash and wrote
`xmp:Label="Yellow"` for class 2 in the round-trip test. Defaults now use
those names; the classic names are accepted when reading. The scheme is a
data structure (`ColorClassScheme`) so it can be loaded from preferences;
not yet wired to a settings file.

## ADR-008 (2026-10-13, accepted): sidecar file name case follows Photo Mechanic

PM wrote `850_7802.XMP` for `850_7802.NEF`. An existing sidecar of either
case is reused; a new one copies the case of the source extension. On APFS
this is cosmetic; on a case-sensitive filesystem it prevents two sidecars.

## ADR-010 (2026-10-13, accepted): a known label beats a disagreeing colour-class number

Photo Mechanic writes `xmp:Label` and `photomechanic:ColorClass` together.
Capture One (verified) and Lightroom (by its documentation) only update
the label. When both exist and disagree, and the label maps to a class in
the scheme, the label wins and the next write repairs the number. An
unrecognised label leaves the number in charge.

## ADR-011 (2026-10-14, accepted): structured XMP properties edited in place with the regex engine, not a quick-xml port

The migration plan said IPTC would come from porting Apature's quick-xml
writer. Building the structured cases (`rdf:Alt`, `rdf:Seq`, `rdf:Bag`,
self-closing `rdf:Description`, namespace declaration) onto the existing
in-place engine took less code, kept one writer, and passes the same
read-back and exiftool checks. Known limits remain those of ADR-004
(comments, multi-Description documents). Revisit if a real sidecar shape
breaks it.

## ADR-012 (2026-10-14, accepted): the assignment file is the stationery format

The owner's `key: value` generator config (date, slug, venue, city, event,
keywords, team, photographer, code lines) is parsed directly, with extra
optional keys (state, country, source, copyright, creator, caption_writer,
credit). One file drives the IPTC pad, the caption template and the code
table in both Apature and the existing Photo Mechanic kit generator.

## ADR-013 (2026-10-14, accepted): ingest is plan-then-run, sequential, never destructive

The planner computes every destination before anything is copied and
refuses collisions (existing files, two sources to one name) and nested
source/destination paths. The runner copies one file at a time (card
readers are the bottleneck), hashes source and copy in full, makes the
secondary copy from the fresh primary, and records a fingerprint in the
"already ingested" ledger only when every destination verified. Erasing a
card is out of scope until an .app bundle and a triple-gated design exist.
Independently written; the shape follows the original Apature's planner,
which the owner authored.

## ADR-014 (2026-10-14, accepted): in-app tabs, state swapped not duplicated

Native NSWindow tabbing would need one window per folder and winit does not
expose it; Photo Mechanic's in-app contact-sheet tabs are the model the
owner asked for. Per-folder state (`TabState`) is moved out of `App` when a
tab is parked and back when it is activated, so every UI module keeps
reading `app.items` and friends. Cost: one `take`/`put` pair of ~20 fields
to keep in sync when fields are added. Open tabs are remembered in prefs.

## ADR-015 (2026-10-14, accepted): Settings is a second window

macOS users expect ⌘, to open a window, not a panel. egui's immediate
viewport gives a real window sharing the context and tokens. Pages:
General, Color classes, Metadata, Ingest, Cache, Shortcuts (read-only until
profiles exist).

## ADR-016 — Card detection by polling `/Volumes` (2026-10-14)

**Decision:** re-read `/Volumes` every 2 s from the UI loop; a new volume
with a `DCIM/` folder counts as a camera card. **Alternatives:** FSEvents
via the `notify` crate (instant, one more dependency and a thread);
DiskArbitration (precise mount events, needs objc bindings). **Why:** one
`read_dir` of three entries every 2 s is free, needs no dependency, and
2 s is below the time it takes to reach for the mouse. Revisit with
`notify` when Live Ingest needs a watcher anyway. **Status:** implemented,
tested with a mounted disk image, not yet with a physical card.

## ADR-017 — Export pixels and metadata (2026-10-14)

**Decisions.** (1) Export does not demosaic RAW itself: a pair exports its JPEG, a
RAW alone exports its largest embedded preview (the camera's rendering,
8256 px on a D850). An opt-in **system RAW decode** renders the RAW with
the operating system's engine instead: on macOS ImageIO through
`/usr/bin/sips`, matched to sRGB, with the camera EXIF carried over and
the orientation tag left for our own rotate step (sips does not bake it,
measured). No new dependency, no licence question, Apple's rendering (the
one Preview and Photos show). It is off by default because it is slow
(several seconds a frame, measured in STATUS.md) and because the camera's
JPEG is what the photographer judged on the back of the camera. Other
platforms fall back to the preview and say so. A built-in demosaic waits
for a permissively licensed one (candidates: `quickraw`, MIT, unverified). (2) The source ICC profile is copied, not converted: an Adobe RGB
JPEG stays Adobe RGB and correctly tagged. Conversion to sRGB needs a CMS
(`moxcms`, BSD-3, is the candidate). (3) IPTC-IIM is written next to XMP
with identical values; over-length values are written whole rather than
truncated, matching Photo Mechanic and exiftool's own behaviour, and
TimeCreated is only written when a zone is known because IIM requires one.
(4) A RAW-only export gets a minimal EXIF block built from the summary so
capture time and camera survive; a JPEG source keeps its whole EXIF with
Orientation reset and pixel dimensions patched. (5) Marks (rating, class,
tag) are not written into delivery files. (6) Export never overwrites.
**Status:** implemented and tested (see STATUS.md M7).

## ADR-018 — Rename and Trash (2026-10-14)

**Decisions.** Rename acts on the whole photo (every variant and the
sidecar), is planned and shown before it runs, refuses collisions, and
runs in two phases so swaps and case changes work with rollback on any
error. There is no undo entry for a rename: the plan preview is the
safeguard, and a second rename reverses it. Trash goes through
NSFileManager on macOS (the `trash` crate's `NsFileManager` method) because
the Finder-scripting default needs an Automation permission and timed out
in testing; the cost is that Finder may not offer Put Back for those
items, which the dialog says. Apature still never deletes.
**Status:** implemented, CLI-tested, dialogs seen.

## ADR-019 — Ingest is its own window (2026-10-15, accepted)
Egui viewport; the sheet stays in the main window so culling continues while a card copies. Headless runs keep it inline. Rejected: a modal in the main window (blocks culling).

## ADR-020 — Delivery and secrets (2026-10-19, accepted)
`apature-deliver` crate: Folder, FTP, FTPS (explicit TLS), SFTP. `.part` then rename. Passwords only in the macOS Keychain (service `dev.mccal.Apature.delivery`, account = target name), never in prefs. Cloud APIs (Drive, Graph) planned with PKCE; until then the desktop sync folder is the cloud path. Rejected: storing credentials in prefs.json.

## ADR-021 — Toasts (2026-10-20, accepted)
Info toasts leave after 6 s; warnings and errors stay until dismissed. Unplug, delivery, ingest and export outcomes post one. Reduce Motion drops fades. Rejected: status-bar-only (messages scrolled away).

## ADR-022 — Live Ingest by polling (2026-10-20, accepted)
Source polled every 2 s; a file is handed over once its size held for two polls. Batches run through the normal ingest. Rejected: FSEvents (unreliable on network and FTP-written folders).

## ADR-023 — Lavender in both appearances (2026-10-19, accepted)
Light accent #5A48C7 / wash #E7E3F8, greys with a violet cast; dark accent #C9BEF8, sheet ground #1C1C20. Darkroom grounds stay neutral. Rejected: system blue in Light (no identity).

## ADR-024 — IBM Plex Sans + Mono (2026-10-19, accepted, not yet embedded)
Chosen in the design system; the app uses system fonts until the files are embedded.

## ADR-025 — JPEG XL through an external cjxl (2026-10-19, accepted)
Lossless transcode of the finished JPEG; macOS cannot encode JXL here and jpegxl-rs needs cmake. Rejected for now: bundling libjxl.

## ADR-026 — Number keys rate by default (2026-10-19, accepted)
Bare 1–5 = rating, ⌘1–8 = class; Photo Mechanic mode available in Settings › Shortcuts.

## ADR-027 — Burst stacks by capture second (2026-10-20, accepted)
Frames within one second group behind the first; off by default. Sub-second EXIF is not read, so one second is the grain.

## ADR-028 — Versioned prefs (2026-10-21, accepted; from Folio STA-5/6/7)
`version` key, step migrations on raw JSON, `prefs.damaged.json` kept on parse failure, backup before the first write after an upgrade, newer files never rewritten.

## ADR-029 — Preview window is optional (2026-10-21, accepted)
Default inline (faster for one screen); Settings switch opens Preview in its own window with full keyboard handling, like Photo Mechanic.


## ADR-030 — Updates install themselves in place (2026-10-26, accepted)
Alic's updater (Tauri's plugin: check, download, replace the running bundle, restart) rebuilt on egui. The payload is the
release's DMG, which the releases already carry, so no new artifact and no signing key is needed: `update_install`
downloads it to a temporary folder, mounts it read-only with `hdiutil`, takes the app out with `ditto` and checks the
copy's `CFBundleIdentifier` before it counts. A running binary cannot be replaced (`ETXTBSY`), so the swap is a generated
`/bin/bash` script that waits for this process to quit, moves the old bundle aside in the same folder, `ditto`s the new
one to the same path, deletes the old app and the download, and opens the new app; the app quits when the script is
ready. Everything is written to `swap.log` next to the script. A failure before the swap changes nothing; a failed copy
puts the old app back. The old build is deleted rather than kept as a backup, as asked; a build that cannot replace
itself (translocated, read-only volume, unwritable folder, or not in a bundle at all) is offered the release page
instead. Rejected: Tauri's updater artifacts (a `.app.tar.gz`, `latest.json` and a minisign key pair for every release,
and a second signing scheme beside codesign); replacing files while the app runs (a second instance and a half-written
bundle); installing without quitting (the app has to exit, so a running ingest, copy, export, send or extraction blocks it).
