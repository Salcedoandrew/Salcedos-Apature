# Changelog

All notable, user-visible changes. Versions follow semantic versioning;
0.x while the app is finding its shape.

## [Unreleased]

### Added
- Tools › Extract JPEG Previews from RAW Photos… (as in Photo Mechanic) saves the largest JPEG inside each selected RAW (or every RAW in the tab when nothing is selected) as a .jpg with the RAW's base name, next to the RAW or in a folder you pick. The picture's bytes are written as they are in the RAW, never re-encoded. Four remembered options control the metadata: caption and keywords from the sidecar or RAW, the RAW's Exif (camera, lens, exposure, capture date, orientation), tag, colour class and rating, and the ICC profile (sRGB when the preview has none). The JPEG is dated to the capture time. A taken name gets a number (nothing is ever replaced, and a camera's NAME.JPG counts), non-RAW photos are skipped, and the RAWs are only read. It runs in the background with progress, Cancel, and a summary of what was extracted and skipped.
- Edit › Select Tagged, Select Untagged, Select by Rating (exactly 1–5 stars, at least 2–4, any rating or none), Select by Color Class (each class by your names, any class or none), Select Cropped and Select RAW+JPEG Pairs pick from the photos showing in the current tab. In the Photo Mechanic layout they take its keys: Select Tagged ⌘T, Select Cropped ⌘⇧C, a colour class ⌘⌃1–8 (⌘⌃9 any, ⌘⌃0 none), a star rating ⌥⌘1–5 (⌥⌘6 any, ⌥⌘0 none).
- File › Copy Photos… (⌘Y) copies the selected photos to a folder you choose (remembered next time): rename on the way with the Rename Photos tokens, bring along the RAW+JPEG partner and the XMP sidecar, make dated subfolders, and choose what happens when a name is taken (skip, add a number, or replace). Files are copied byte for byte, checked, and keep their created and modified dates; the originals are never touched. It runs in the background with a progress bar and Cancel, and can write the loaded assignment pad into the copies' XMP sidecars.
- Photo Mechanic's keyboard shortcuts are now the default. If you are coming from Photo Mechanic, your fingers already know where things are: ⌘1–⌘8 for colour classes, ⌃1–⌃5 for stars, T to tag, ⌘G to ingest, ⌘M to rename. Like the old keys better? Choose Apature classic in Settings › Shortcuts and Apature will remember it. The README has a side-by-side table of both.
- In the Photo Mechanic layout every menu key is used only once: Close Window moves to ⌘⇧W and Minimize has no key, so ⌘W closes the tab and ⌘M renames, just like Photo Mechanic. Changing the layout now updates the menu bar right away, and Live Ingest in the menus works again.
- Settings › Accent: six soft accent presets (Lavender, Sage, Sky, Peach, Rose, Mist) applied to selection, links and focus in both appearances.
- A new app icon (the modern A) with three optional alternates (photo frame, aperture, field), every size and `.icns`, generated from `tools/apature_icons.py` with the accent as a parameter.
- Press kit in `docs/marketing`: hero images, social preview, feature cards, copy kit; `tools/marketing-shots.sh` recaptures the screenshot set.
- Check for updates: once a day, or Apature › Check for Updates…, a toast offers the newest release's download. Settings › General can turn it off or include betas.
- Previews for more RAW formats, and bigger ones. Apature now finds the camera's own JPEG in Fujifilm RAF, Canon CR3 and CRW, Panasonic RW2 and Leica RWL, Olympus ORF (from the maker note), Sony SRF and SR2 and Sigma X3F, and shows the largest one by default. Hasselblad 3FR/FFF and Phase One IIQ files carry no JPEG, so their built-in RGB thumbnail is used instead. CRW, SRF, SR2, RWL, FFF and X3F files now show up in folders.
- `apature-cli preview extract FILE` writes the largest embedded preview out untouched (`--prefer smallest` or a pixel size, `--max-bytes`, `--list`), and `apature-cli preview list FILE` lists them all.

### Changed
- Copy Photos… has Photo Mechanic's Destination choices: Original folder (next to each original, where a taken name always gets a number and nothing is ever replaced), Create subfolder (SELECTS by default, inside each original's folder), This folder plus an optional sub-path, or Always pick destination (asks when you press Copy). "Open destination as a Contact Sheet" opens the copies in a new tab afterwards. All of it is remembered, and dated subfolders, renaming and the name-clash choice apply under whichever destination you pick. The text boxes are white with dark text, and the rename box starts empty with its tokens behind a Variables… button.
- Canon CR2 and some DNG files used to show a reduced or the wrong picture (sometimes the raw sensor data itself); lossless raw streams are no longer mistaken for previews, so the camera's real preview is used.
- Fujifilm RAF files now show the right camera, lens, exposure and orientation.
- Screenshots now show a real storm-damage assignment instead of test frames.
- Example outlet names removed from the export preset hint and test fixtures.

### Fixed
- Single-key shortcuts now work on the Mac: T tags, X rejects, C crops, ⌥C removes a crop, V compares, Z zooms, B brightens, L loupes, S opens a burst and the bare number keys rate (or set the class, per Settings), in both layouts. Before, macOS only passed ⌘ and ⌃ shortcuts to the menus, so these showed in the menus but did nothing. Each press acts exactly once, and nothing fires while you are typing in a caption, the rename box or a dialog field.
- Marking never changes your JPEG files any more. Ratings, colour classes, tags, rejects, captions and the assignment pad for a JPEG now go to an XMP sidecar beside it (`name.xmp`, or the RAW's sidecar for a RAW+JPEG pair), the same place RAW marks already go. Marks earlier versions wrote inside JPEGs still show; a newer mark in the sidecar wins. If you want marks written into the JPEG too (Photo Mechanic's habit), turn on Settings › General › Write marks into JPEG files; it is off for everyone, including existing users, and when on it now keeps the file's created and modified dates.
- Clearing a caption, headline, keywords or any other caption field on a JPEG now sticks. With marks kept in sidecars, a cleared field used to come back from the text embedded in the JPEG the next time the folder opened; the sidecar now records the clear, and the JPEG itself is still not touched. Photos whose JPEG carries older marks than the sidecar no longer show a conflict marker either: the sidecar is the record (conflicts are still reported if you turn on Write marks into JPEG files).
- A cancelled Copy Photos shows how far it got instead of 100 %, and Compare's status names the right key to stop it (V in the Photo Mechanic layout).
- Canon CR3 files now show their capture date, camera and exposure, so Copy Photos files them in the right dated folder and ingest names using the date work for CR3 too (before, CR3 fell back to the file's date).

## [0.1.0] — 2026-10-21

First shareable build, ad-hoc signed: on first launch right-click › Open, or allow it under System Settings › Privacy & Security. Everything below is new.

### Ingest
- Card detection on mount, Ingest in its own window, day chips, a live
  example of the names the templates will produce, MB/s and time left.
- Templates with Photo Mechanic's variable names plus camera, time, weekday,
  folder, destination and ingest-day variables; persistent `{seqn}` per
  headline.
- SHA-256 verification of every copy at every destination; a ledger so a
  card is never copied twice; `-1, -2` suffix option; eject when verified.
- Keep the card's folders; copy voice memos and sidecars; a backup
  destination; GPS for every photo; the sheet opens while the copy runs.
- Live Ingest: a watched folder, files picked up once their size settles.

### Cull
- Ratings, colour classes, tags, filters, sort, tabs, undo.
- Preview fit and 1:1 with one shared magnification, compare up to four,
  loupe, view-only exposure boost, burst stacks.
- Rotate and crop stored in the sidecar and applied on export, with ratio
  presets and thirds or golden-section guides.

### Caption
- Assignment pad, code replacements with an on/off switch, keywords as
  chips with recent values, GPS, Title, Caption writer, Instructions,
  Transmission reference, Save & next, Save to selection, copy and paste
  metadata.

### Export and delivery
- JPEG or JPEG XL (via cjxl), resize, quality, EXIF, XMP and IPTC-IIM kept,
  naming templates, presets per outlet.
- Delivery to a folder, FTP, FTPS or SFTP; passwords in the macOS Keychain;
  three tries per file; a per-photo sent ledger in the inspector; resend
  failed.

### App
- Two appearances with a lavender accent, Reduce Motion, text scaling,
  higher contrast, keyboard focus rings, every function in a menu.
- Folder strip at the top, window title names the folder, toasts, unplug
  detection, a menu bar extra with live status, About with notices, a log
  file, versioned settings with a damaged-file backup.

### Known limitations
- No screen-reader support yet.
- FTPS and SFTP are untested against a real desk; Google Drive and OneDrive
  sign-in are not built (use their desktop sync folder with a Folder target).
- JPEG XL needs Homebrew's jpeg-xl installed.
- Rotation and crop are applied on export, not shown as a crop on the sheet.
- macOS only.
