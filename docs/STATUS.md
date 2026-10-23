# Status

Last updated 2026-10-13. Vocabulary: **implemented** = code exists;
**tested** = automated test passes or a manual run is recorded here;
**verified** = tested against an external reference (exiftool, another
app, a measurement); **production-ready** is not claimed for anything.

## Where Apature is on 2026-10-21 (read this first; older sections are history)

Daily loop complete and verified on real shoots: card in → verified ingest
(day chips, live example name, MB/s, sheet opens while copying, Live Ingest)
→ cull (ratings, classes, tags, burst stacks, loupe, exposure boost, rotate,
crop) → caption (pad, codes, keywords, GPS, recent values, save to
selection) → export (presets, JPEG or JPEG XL) → deliver (Folder, FTP,
FTPS, SFTP; Keychain; retry; sent ledger). Two appearances, lavender accent,
Reduce Motion, text scale, contrast, focus rings. Menus complete, About,
menu bar extra, toasts, unplug detection, log file, versioned prefs.

Measured: 134 NEF / 6.0 GB verified ingest from a Z 6 II card; headless
harness captures every workspace. Tests: 94. CI: Linux + macOS.

Known limitations (current): no VoiceOver tree; FTPS/SFTP untested against a
real desk; Google Drive / OneDrive API sign-in waits on client IDs; no
AF-point overlay (maker note); JPEG XL needs Homebrew cjxl; ad-hoc signed
bundle only (no Developer ID yet); thumbnail priority on very large folders
unmeasured; Apature Pocket is a Python prototype on a branch, not merged.

Plan and handoff: docs/PLAN.md. Decisions: docs/DECISIONS.md (ADR-019+ added
2026-10-21). Cross-project lessons: docs/cross-project/.

## Engine

| feature | status | evidence |
|---|---|---|
| Folder scan, RAW+JPEG pairing, sort | tested | unit tests; `apature-cli scan` on 3 folders |
| TIFF/IFD reader | tested | unit tests incl. hostile input |
| Embedded preview discovery (IFD chain, SubIFDs, Exif IFD, Nikon MakerNote PreviewIFD, plain JPEG) | **verified** | identical previews and byte counts to exiftool on D850 NEF; synthetic TIFF tests |
| JPEG decode with DCT scaling, orientation, thumbnail | tested | round-trip tests; D850 NEF/JPG runs |
| Disk thumbnail cache (key: path, size, mtime, size) | tested | unit test; bench warm runs |
| EXIF summary (make/model/lens/date/exposure/orientation) from TIFF and JPEG APP1 | **verified** | matches exiftool on D850 NEF |
| XMP sidecar read/write, in-place, atomic, read-back checked | **verified** | exiftool; **Photo Mechanic 2026.6 round trip both directions** (NEF-only and pairs); **Capture One 16.8 round trip both directions** (reads sidecar + embedded; its sidecar rewrite handled, stale ColorClass resolved by label); see `docs/research/metadata-safety.md` |
| Embedded-XMP read **and write** in JPEG, pair policy (sidecar + embedded), conflict report | **verified** | unit tests; exiftool on a D850 JPEG; Photo Mechanic shows embedded marks for pairs |
| Verified copy (temp → SHA-256 both sides → rename; never overwrite; never delete source) | tested | unit test only; no card-reader run |
| CR3 / HEIC / RAF-with-no-TIFF previews | **not implemented** | returns "no embedded JPEG" |
| RAW decode fallback when no preview | **not implemented** | |
| ICC colour management | **not implemented** | previews shown as sRGB |
| IPTC Core fields (title, headline, caption, keywords, creator, credit, source, copyright, caption writer, city, state, country, country code, sublocation, date created, instructions, transmission ref, usage terms) read/written in place incl. rdf:Alt/Seq/Bag | **verified** | unit tests on Lightroom/PM-style sidecars and self-closing Descriptions; exiftool validates sidecar and JPEG after a write; marks survive |
| Code replacements (PM file format, `\code\`, `{iptcdow} {iptcmonthnameap} {iptcday} {iptcyear4} {credit} {headline}`) | **verified** | unit tests; `\cf\ tees off on No. 1\tail\` expands to the full AP caption |
| Assignment stationery from the owner's `key: value` generator file → IPTC pad, caption template, code table | **verified** | unit tests; `apature-cli pad` and `caption` on the scratch pair, exiftool read-back |
| Ingest: naming templates (PM variable names), plan with collision refusal (on disk and within the plan), nested-path refusal, file filter, "only new" fingerprint ledger, sequential verified copies to primary + secondary (secondary from the fresh primary), per-file events, cancellation, report, ledger updated only when every destination verified | **verified** | 8 unit tests incl. a failing destination and cancellation; `apature-cli ingest` from a fake card to two destinations: every file verified, second run skipped via ledger, card untouched |
| Catalog | not started | Phase 4+ |

## Desktop shell (`apps/desktop`, M2, 2026-10-13)

The designed shell replaced the probe. Verified by automated screenshots of
the running app in both appearances (`apature <folder> --theme dark|light
--screenshot out.png`) and by querying the native menu bar through System
Events.

| feature | status |
|---|---|
| Tokens for Apature Dark and Light, following the system appearance; egui widgets restyled through them | implemented, **verified** in screenshots |
| Native menu bar (Apature, File, Edit, View, Image, Window, Help) with accelerators; ⌘ shortcuts owned by the menu | implemented, **verified** via System Events (`Apple, File, Edit, View, Image, Window, Help`; View and Image items listed) |
| Toolbar 48 pt: sidebar toggle, workspace switcher (Caption/Ingest disabled with the milestone named), Filter and Sort disabled with reasons, size slider, decode counter, inspector toggle | implemented, verified in screenshots |
| Source-list sidebar: Favorites (add/remove), Recent (10), Folders tree (Home and each volume; favorites expand too; lazy background reads; click opens here, double-click a new tab), Collections placeholder; 26 pt rows; resizable 200–320 with memory | implemented, verified in screenshots (`--sidebar-shot`) |
| Contact sheet: virtual grid, letterboxed cells, class bar, chip, stars, tag, pair badge, focus ring, ellipsised filenames; cells carry accessibility names | implemented, verified in screenshots; **VoiceOver not yet tested** |
| Inspector: sharp preview on top with honest size label, Capture / Marks (editable) / Caption (M4) / Files sections, resizable 280–420 with memory | implemented, verified (preview arrives at 1024 px in ~19 ms) |
| Filmstrip 88 pt (⇧F), Preview workspace with overlay and ±1 prefetch, status bar 28 pt | implemented, run manually |
| Panel memory and recent/favorites in `~/Library/Application Support/dev.mccal.Apature/prefs.json` | implemented, run manually |
| Reduce Motion read at startup → animation time 0 | implemented, not tested with the setting on |
| App icon (assets/icon.svg → icon-256.png, Apature.icns) as window/Dock icon | implemented; Dock shows it when launched; the app-menu title still reads as the launching process until an .app bundle exists (M6) |
| SF Pro / SF Mono at runtime | **not loaded**: the system variable fonts fail ab_glyph validation, so egui's bundled fonts are used (visible in screenshots) |
| Selection separate from focus: click, ⇧-range in view order, ⌘-toggle, ⌘-arrows keep, ⌘A/⌘⇧A; marks apply to the selection when the focused item is in it; "mixed" shown in the inspector | implemented, **unit-tested** (`selection.rs`), verified in a demo screenshot |
| Filters: rating ≥, hide rejected, classes, tag, pairs; chips in the toolbar; "showing x of y" | implemented, unit-tested (`filter.rs`), verified in screenshot |
| Sort: name, capture time, rating, class, reverse; EXIF read for all items at open | implemented, unit-tested |
| Undo/redo, one entry per action (⌘Z/⌘⇧Z, Edit menu) | implemented, unit-tested (`undo.rs`); writes files on undo |
| Advance after mark (Image menu toggle, shown in the status bar) | implemented |
| Ingest workspace (⌘⇧4): volumes + folder as source, subfolders, only-new, file filter, primary/secondary pickers, folder and rename templates, headline (prefilled from the assignment), sequence start, optional pad-after-ingest, plan rebuilt on a worker thread on every change with collisions in red, Start disabled until clean, per-file ✓/✕ list with error text, cancel after current file, report, "Open the ingested folder" | implemented, verified in a screenshot (plan state); the run path verified through the CLI, not clicked in the GUI |
| Drag and drop: a folder dropped on the window opens as a contact sheet; a file opens its folder with that photo focused; overlay while hovering | implemented, **not yet exercised** (needs a real drag; eframe's dropped_files API) |
| Toolbar icons drawn with the painter (sidebar, inspector, funnel) instead of Unicode glyphs; app icon redrawn (variant B), bracket variant kept in assets | implemented, verified in screenshots |
| Tabs: one contact sheet per tab (view, selection, filter, undo, caption draft per tab); ⌘O/⌘T open in a new tab, ⌘W closes, ⌘⇧]/⌘⇧[ cycle, strip under the toolbar with close boxes and "+"; open tabs restored at launch; a folder already open just switches to its tab | implemented, verified in a screenshot with three tabs |
| Settings window (⌘,): appearance (system/dark/light), advance after mark, open folder after ingest, default thumbnail size; colour-class names and colours (menu items follow); assignment and code files, code delimiter; ingest defaults; thumbnail cache size and clear; shortcut table | implemented; **layout not yet reviewed** (child windows cannot be screenshotted by the app itself) |
| `Apature.app` bundle via `scripts/bundle.sh` (Info.plist with identifier `dev.mccal.apature`, icon, folder document type, ad-hoc codesign; `--install` copies to /Applications) | implemented, **verified**: launched, process and menu title read "Apature", bundle identifier reported by System Events, quit cleanly |
| Motion: sidebar/inspector/filmstrip collapse and reveal eased; tab highlight slides and the new sheet fades in (160 ms); Preview grows out of the focused cell; filter chips, drop overlay and status text fade in (120 ms); ingest progress eases; all durations 0 under Reduce Motion (`App::anim`) | implemented; motion cannot be judged from stills, so **unreviewed by eye**; builds, runs, 60 tests |
| Caption workspace (⌘⇧3): photo left, fields right, assignment header, Load assignment / Apply pad to N / Insert template, inline `\code\` expansion with autocomplete hint, ⌘⏎ save and next, autosave on navigation, Revert | implemented, verified in a screenshot; writes verified via the CLI path |

## M10 Parity items: copy/paste metadata, eject, auto-suffix (2026-10-15)

**Copy / Paste Metadata** (Edit menu, ⌥⌘C / ⌥⌘V): copies the focused
photo's marks and IPTC into an internal clipboard; paste applies the
marks to the selection through the undo stack and writes the IPTC in
place (sidecar for RAW, embedded for JPEG, both for pairs); the status
line reports photos and files. Not undoable for IPTC. Implemented, built,
not yet driven by hand.

**Ingest collision policy** (`CollisionPolicy::Suffix`, Photo Mechanic's
AutoIncSuffix): when a destination name is taken, the plan walks
`name`, `stem-1.ext`, `stem-2.ext`… until free at every destination and
within the plan, and counts them as "renamed with a suffix". Refuse stays
the default. Tested: unit test, and the CLI ingested the same card twice
into one folder, refusing without `--suffix` (4 collisions) and producing
`850_7783-1.NEF` etc. with it, all verified.

**Eject** after ingest: a pref "Eject the card when every file has
verified" (default off) and an "Eject the card" button on the Progress
card for sources under /Volumes; runs `diskutil unmount` off the UI
thread. Implemented; not yet tried on a physical card (the DMG was not
mounted during this pass).

## M9 Rename in place and Move to Trash (2026-10-14)

`apature-core::fileops`: a rename plan names every file of each photo
(RAW, JPEG, XMP sidecar) and refuses any collision (existing file, or two
photos to one name); execution is two-phase (all to temporary names, then
to targets) so swaps and case-only changes work, and any failure rolls
back. Trash moves all of a photo's files to the system Trash through
NSFileManager (the crate's Finder-scripting default timed out here and
would need an automation prompt); nothing is ever deleted. Desktop: File ›
Rename Photos… (⌘⇧R) opens a modal with the naming template (same tokens
as ingest and export, last template remembered), sequence start, the full
old → new list and any collisions, Enter or Rename applies and the items
update in place with the view re-sorted; File › Move to Trash (⌘⌫) asks
first, listing the photos and the file count, then reloads the folder
with the focus kept in place. Shortcuts are held while a dialog is open.
CLI: `apature-cli rename <paths> --template T [--seq-start N] [--dry-run]`,
`apature-cli trash <paths>`. Tested: CLI rename of three pairs with
sidecars (marks still read afterwards), collision refusal, a trashed pair
left the folder; both dialogs seen in screenshots. Not done: undo for
rename (the dialog shows the plan instead), copy/paste metadata, Trash
"Put Back" is up to macOS for NSFileManager-trashed files.

## M8 Preview 1:1 and compare (2026-10-14)

Preview workspace: **Z** (View › Zoom 1:1 / Fit, or double-click at the
point of interest) toggles between fit and 1:1, where one pixel of the
decoded preview is one physical screen pixel; drag or scroll to pan,
clamped to the image edges; Esc steps back to fit, then to the sheet.
1:1 asks the worker for the full file (capped at the GPU's largest
texture; the worker now reports whether a decode is the file's full
resolution and never uploads more than requested). Zoom eases between the
two rects over 160 ms. **C** (View › Compare Selected) shows the
selection side by side: two panes split the viewport, three or four make
a 2×2 grid; zoom and pan are shared so the same crop is compared; clicking
a pane focuses that photo; marks apply to the focused one. A lone photo is
compared with its next neighbour. Previews are now evicted by a byte
budget (768 MB of texture) as well as count, keeping the focused photo,
its neighbours and the compare set.

Seen in screenshots (`--preview`, `--zoom11`, `--compare` flags): fit,
1:1 of the 8256 px D850 preview, and a three-up compare. Not watched by
eye: drag panning, the zoom ease, scroll panning. The status bar now
truncates the status text instead of running into the frame stats.

## M7 Export (2026-10-14, tested)

New crate `apature-export` and Export workspace (⌘⇧5, File › Export… ⌘E):
pixels from the JPEG of a pair or a RAW's largest embedded preview (no RAW
decode), rotated upright, resized to a long side (default 2048) with
Lanczos3 after DCT-domain downscaling, re-encoded at quality 90. Metadata:
EXIF copied from a JPEG source with Orientation reset to 1 and
PixelX/YDimension patched; a RAW-only source gets a minimal EXIF block
(make, model, lens, capture time, exposure, ISO, focal length) built by
`exif_write.rs`; XMP packet with the IPTC Core fields; **IPTC-IIM** APP13
block (`iim.rs`, UTF-8 declared, TimeCreated only when a zone is known);
ICC profile segments copied unchanged. Atomic write, never overwrites.
Naming uses the ingest template engine (`100726-SPO-BGLF-{seqn:3}`);
with an assignment loaded its fields fill what the file lacks, the file's
own caption wins. CLI: `apature-cli export <files|folder> --out <dir>
[--long 2048] [--quality 90] [--rename tpl] [--assignment file]`.

Measured (M1 Max class laptop, release): 114 captioned 2048 px JPEGs →
2048 px in 1.5 s via the CLI (4 threads) and 2.6 s in the app;
three D850 NEF+JPG pairs (45 MP) ~0.5 s each; NEF-only via embedded
preview 0.3 s. `exiftool -validate` on every output: OK, except two
*minor* length warnings when the owner's credit line (36 bytes) and slug
(35 bytes) exceed the IIM 32-byte limits; values are written in full, as
Photo Mechanic does. Verified: IIM and XMP read back identical to the
source captions; a source tagged Orientation 6 exports upright at 666×1000
with matching EXIF dimensions; sRGB profile carried.

**RAW → JPEG (opt-in, macOS):** "Decode RAW-only photos with the macOS RAW
engine" in the Export workspace, `--decode-raw` in the CLI. The RAW is
rendered by ImageIO through `/usr/bin/sips` at full resolution (8256×5504
from a D850 NEF), matched to sRGB, then goes through the same resize and
metadata path with a clean EXIF block built from the camera fields.
Measured: about 8 s per NEF on this machine (sips' own time; the
embedded-preview path takes 0.3 s), so it is off by default. A NEF tagged
Orientation 6 came out upright at 1365×2048; `exiftool -validate` OK on
both outputs. Other platforms fall back to the preview with a log line.
A built-in demosaic needs a permissively licensed crate (ADR-017).

Not done: colour conversion to sRGB for Adobe RGB JPEG sources (profile
is kept so the file stays correct), HEIC sources, watermark, export of
marks (rating/class are deliberately not written to delivery files),
per-outlet presets. `--export-run --export-dest <dir>` runs the app path
headlessly.

## Motion slice 1 and card-mount detection (2026-10-14)

Implemented: thumbnail arrival fade (120 ms), selection ring fade, focus
ring slide after a keyboard move (cuts when the old cell is more than four
cells away), ingest rows staggered 15 ms apart on start, ✓ colour fade,
progress bar driven by verified bytes, 600 ms accent flash on the Progress
card when the run ends. All durations go to 0 under Reduce Motion
(`App::anim`). Seen in screenshots: the arrival fade and the finished
ingest; not yet watched by eye: selection fade and focus slide (no key
input in screenshot mode).

Card mount: `/Volumes` is polled every 2 s (no new dependency; FSEvents can
replace it later). A newly mounted volume with `DCIM/` sets the status
"Card mounted: NAME" and, when Settings › Ingest "When a camera card
mounts, open Ingest" is on (default), switches to Ingest with that card as
the source. Tested by attaching the "NIKON Z6" disk image while the app
ran: workspace switched, plan built (6 skipped, already in the ledger).

## End-to-end ingest in the app (2026-10-14, tested)

`apature --ingest-run --ingest-source <card> --ingest-dest <dir>
[--screenshot out.png]` runs the real desktop path (plan on a worker,
copy+verify, pad, open-after-ingest) without a mouse and prints the report.
Against a mounted FAT32 disk image named "NIKON Z6" (DCIM/100NZ6_2 and
101NZ6_2, three NEF+JPG pairs, AppleDouble `._` files present): 6 files
planned, 6 verified, 242.6 MB in 2.2 s; SHA-256 of source and copy match;
`._` files ignored; the ingested folder opened as a new tab with the marks
from the embedded XMP showing; the second run planned 0 files (ledger).
CLI with the owner's PM templates produced
`2025/04 April/Apr 10 100726 SPO BGLF/250410100726 SPO BGLF001.NEF` at both
destinations (485 MB in 4.4 s). Not yet tried: a physical card over USB/SD
reader (expected identical; the volume list is `/Volumes`).

Motion and polish research: `docs/design/MOTION_RESEARCH.md`.

## Ingest variables (2026-10-14)

Photo Mechanic's spellings (`{year4}` `{iyr2}` `{month0}` `{imn0}`
`{iptcmonth0}` `{day0}` `{idy0}` `{iptcday0}` `{monthname}` `{mnn3}`
`{iptcmonthname3}` `{hour}` `{minute}` `{second}` …) are accepted so the
owner's existing folder and rename templates paste in unchanged; tested
against the exact strings in his presets.

## M6a notes (2026-10-14): tabs and settings

* Tabs are in-app (Photo Mechanic style), not NSWindow tabs: the app is one
  egui window, and per-folder state is swapped in and out of the `App`
  fields (`TabState`) so the UI code did not change. Parked tabs keep their
  textures; many large tabs will hold GPU memory until closed.
* The Settings window is an egui immediate viewport (a real second window).
* Changing a colour-class name renames the Image › Color Class menu items
  live; existing sidecars keep their stored label until rewritten.

## M5 notes (2026-10-14)

* Card auto-detect/unmount and erase-source are deliberately absent; see
  `docs/research/ingest-ui.md` for the gates they need.
* Secondary copies come from the primary copy, not the card, so the card is
  read once; both copies are independently hashed.
* The ledger lives in `~/Library/Application Support/dev.mccal.Apature/ingested.json`.

## M4 notes (2026-10-14)

* Structured XMP (Alt/Seq/Bag) is edited with the same regex-based in-place
  approach as marks rather than the planned quick-xml port (ADR-011). It
  passed the same tests and exiftool validation; the port stays an option
  if a sidecar shape defeats it.
* IPTC edits have no undo yet (marks do). The draft's Revert covers the
  unsaved case only.
* IPTC-IIM (APP13) is not written into JPEGs; readers that ignore XMP will
  not see captions. Photo Mechanic, Lightroom, Capture One and exiftool
  read XMP first.
* The caption field is egui's TextEdit: no system spell-check or dictation.
  Watch for this being the reason to add a native text view.
* Photo Mechanic's display of our captions was not visually checked this
  session (exiftool read-back was); worth a ten-minute look.

## M3 notes (2026-10-14)

Batch writes are synchronous on the UI thread: marking 100 selected pairs
writes 200 files (~ms each). Acceptable for now; a background writer with a
progress row is the fix when it is felt. Workspace shortcuts moved to ⌘⇧1/⌘⇧2
because ⌘1–5 are Photo Mechanic's ratings.

## Previous probe (superseded)

| feature | status |
|---|---|
| Open folder (⌘O / button, native panel, CLI arg) | implemented, run manually |
| Virtual contact sheet, cell-size slider and +/− | implemented, run manually |
| Viewport-priority background decoding, texture eviction beyond 12 rows | implemented; eviction **not observed** under scrolling |
| Keyboard: arrows, Home/End, PgUp/PgDn, Enter/Space/Esc | implemented, run manually |
| Marks: 1–8 class, 0 clear, ⌘0–5 stars, X reject, T tag → sidecar | implemented; sidecar output verified via CLI path, GUI path run manually |
| Full-screen preview with ±1 prefetch, LRU of 6 | implemented, run manually |
| Status line with EXIF, marks, decode and frame stats | implemented |
| Multi-select, filters, sort options | **not implemented** |
| Menu bar, sidebar, inspector, light theme, VoiceOver names, Reduce Motion | **not implemented** (see `docs/design/MACOS_UI_REQUIREMENTS.md`) |

The probe could not be screenshotted; the M2 shell can (see above).

## Verification run (2026-10-13)

* `cargo fmt --check`: clean. `cargo clippy --all-targets`: 0 warnings.
* `cargo test`: 28 passed, 0 failed (one test skips when exiftool is absent;
  it ran here).
* `cargo build --release`: ok.
* Originals: the three NEF+JPG fixtures are size-identical to the Google
  Drive originals; the 114-file assignment folder gained no files and no
  sidecars; all access is read-only `mmap`. Byte hashes against the cloud
  originals were not computed (would re-download 165 MB).

## Known limitations and risks (M3-era, superseded by the section at the top)

## Next priorities

See docs/PLAN.md.
