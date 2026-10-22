# Changelog

All notable, user-visible changes. Versions follow semantic versioning;
0.x while the app is finding its shape.

## [Unreleased]

### Added
- Photo Mechanic's keyboard shortcuts are now the default. If you are coming from Photo Mechanic, your fingers already know where things are: ⌘1–⌘8 for colour classes, ⌃1–⌃5 for stars, T to tag, ⌘G to ingest, ⌘M to rename. Like the old keys better? Choose Apature classic in Settings › Shortcuts and Apature will remember it. The README has a side-by-side table of both.
- Settings › Accent: six soft accent presets (Lavender, Sage, Sky, Peach, Rose, Mist) applied to selection, links and focus in both appearances.
- A new app icon (the modern A) with three optional alternates (photo frame, aperture, field), every size and `.icns`, generated from `tools/apature_icons.py` with the accent as a parameter.
- Press kit in `docs/marketing`: hero images, social preview, feature cards, copy kit; `tools/marketing-shots.sh` recaptures the screenshot set.
- Check for updates: once a day, or Apature › Check for Updates…, a toast offers the newest release's download. Settings › General can turn it off or include betas.

### Changed
- Screenshots now show a real storm-damage assignment instead of test frames.
- Example outlet names removed from the export preset hint and test fixtures.

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
