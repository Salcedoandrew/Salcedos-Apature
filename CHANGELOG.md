# Changelog

All notable, user-visible changes. Versions follow semantic versioning;
0.x while the app is finding its shape.

## [Unreleased]

### Docs
- Added an AI disclosure to the README: this edition was built with help from AI coding assistants and tested by Andrew Salcedo.


### Added
- A Zoom section in Preview's right-hand panel, below Info, like Photo Mechanic's: a "Zoom:" checkbox, a slider with the steps ¼, ½, ¾, 1, 2 … 8× (1 = one image pixel per screen pixel) and Lock scrolling. Ticking Zoom jumps straight to the slider's level; unticking goes back to fit; dragging the slider rescales live around the middle of the view. Unticking and ticking again returns to the same spot and level. In Preview you can also click a point to zoom in on it, and click again (without dragging) to go back to fit. While zoomed, drag to move around (open hand, closed while dragging); the cursor is a magnifier where a click zooms. Lock scrolling (only while zoomed) keeps the same spot when you move to the next or previous photo; when it's off, each photo opens centred. The checkbox and slider follow the keyboard zoom and ⌘-scroll. The section appears only in Preview, not in the contact sheet. Scrolling no longer pans a zoomed photo; drag instead. In single view a double-click no longer toggles zoom (a single click does); Compare keeps double-click.
- The Info section shows more camera details when the photo has them, following Photo Mechanic's info panel: Firmware, Exp. Comp., Flash Comp., Program, Focus Mode, White Bal., ICC Profile, Contrast, Sharpening and Quality (for example "-0.3 EV", "Manual", "AI Servo AF", "Daylight", "sRGB", "RAW"). They come from the standard EXIF fields and, for Canon, the maker notes. A row only appears when the file has that value. ISO, aperture, shutter speed and focal length stay on the Exposure line. All rows in Info, including the file rows, now sit in one table with even spacing. Files are only read, never changed.
- The Copy Photos progress dialog shows how long the copy has taken and how fast it is going, in a small Time / Speed table like Photo Mechanic's (for example "1 min 12 sec" and "268 MB/sec"). Both update live while copying (speed is the average so far) and show the final time and average speed when the copy finishes. Copying itself is unchanged. Each copy is also recorded in apature.log: a start line (destination, file count, total bytes), an end line (done, cancelled or failed; files and bytes copied; time; average speed) and one line per file that failed to copy, with the reason. Successful files are not logged one by one.
- About Apature credits the original: "Apature, originally created by McCal-Codes" with a link to the upstream project, then "This edition by Andrew Salcedo" with a link to this fork and a short list of the changes in this edition. The copyright, licence and third-party notices are unchanged. Help › Apature on GitHub now opens this fork. The README starts with the same "About this fork" credits. `--about-dialog` opens About at start for screenshots.
- Test builds for Intel Macs too: `scripts/dmg.sh` builds one universal Apature.app (Apple silicon and Intel) and a drag-to-Applications disk image, `Apature-<version>-test-<build>-universal.dmg`, for GitHub pre-releases. `scripts/bundle.sh` accepts `APP` and `APATURE_BINARY` so it can bundle that binary without touching your local build.
- Help › Report a Bug… lets testers report a problem on GitHub: a title (required), what happened, steps to reproduce and what you expected, in white fields like the other Photo Mechanic-style dialogs. "Include system info" (on by default) shows exactly what is added: Apature's version, build and commit, the macOS version, the CPU, the shortcut layout and the focused photo's file type (its extension only, never a file name, folder or metadata). Submit opens a prefilled new issue in your browser (you sign in to GitHub there; Apature holds no token); a report too long for a link is trimmed and the full text put on the clipboard. Copy to Clipboard copies the report as Markdown. While the dialog is open, ⌘A selects the field's text and no key reaches the photos behind it.
- G closes Preview (full-screen or in the window) and returns to the contact sheet with the photo still focused and scrolled into view, like Esc, in both shortcut layouts. In the crop tool G still cycles the guides, ⌘G is still Ingest…, and G typed in a text field stays text.
- A folder tree in the sidebar, like Photo Mechanic's Navigator. "Drives & cards" is now Folders: your home folder first, then every mounted drive and card, and each favorite can be opened the same way with its triangle. Click a folder to show it in the current tab; double-click to open it in a new tab. The folder you are looking at is highlighted, the folders above it open, and it scrolls into view. Subfolders are read in the background the moment you open a row, so a slow network drive or a sleeping disk never freezes the window. Hidden folders (names starting with a dot, ones Finder hides such as Library) and packages like apps and photo libraries are left out. Right-click a folder to open it in a new tab, reveal it in Finder or add it to Favorites. After you click in the tree, the arrow keys move through it (→ opens a folder, ← closes it, Return shows it); click the photos again and the arrows cull as usual. Which folders are open is remembered, and a card appearing or going away updates the list. The tree only looks: it never creates, renames or deletes folders.
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
- The left sidebar (Favorites, Recent and the folder tree) is hidden in Preview so the photo gets the room, and comes back as it was (open folders, scroll position, width) in the contact sheet. The sidebar button or ⌃⌘S still shows it in Preview if you want it there; that choice lasts until you quit and doesn't change the contact sheet's setting.
- The info panel's Capture and Files sections are now one section, called Info. It shows Camera, Lens, Exposure and Taken, followed by the file rows that used to be under Files (the RAW / JPEG / HEIC file and its size, Sidecar, and Conflict, Sent and Edits when they apply), with the same labels and behaviour. The separate Files heading is gone.
- This edition now lives in its own repository, Salcedo's Apature (github.com/Salcedoandrew/salcedos-apature). Check for Updates, Help › Report a Bug…, Help › Apature on GitHub and the "This edition by" link in About all point there, and the README and PRIVACY.md name it. The credit to the original Apature by McCal-Codes, the copyright and the licence are unchanged.
- Check for Updates now looks at this fork's releases (github.com/Salcedoandrew/apature, the same repository Help › Report a Bug… uses) instead of the original project's. Test builds call themselves `<version>-test.<build>`, like their release tags, and a test build is offered any newer test build (by build number, so test.160 beats test.99) or a newer stable, never the same or an older one, even with "Include beta releases" off. Drafts are skipped, the newest suitable release wins whatever order GitHub lists them in, and Download opens the `-universal.dmg` when a release has one. For testing, `APATURE_UPDATE_BUILD=<n>` makes the check pretend to be build n, and `--check-updates` prints which version and repository it checked.
- Copy Photos… has Photo Mechanic's Destination choices: Original folder (next to each original, where a taken name always gets a number and nothing is ever replaced), Create subfolder (SELECTS by default, inside each original's folder), This folder plus an optional sub-path, or Always pick destination (asks when you press Copy). "Open destination as a Contact Sheet" opens the copies in a new tab afterwards. All of it is remembered, and dated subfolders, renaming and the name-clash choice apply under whichever destination you pick. The text boxes are white with dark text, and the rename box starts empty with its tokens behind a Variables… button.
- Canon CR2 and some DNG files used to show a reduced or the wrong picture (sometimes the raw sensor data itself); lossless raw streams are no longer mistaken for previews, so the camera's real preview is used.
- Fujifilm RAF files now show the right camera, lens, exposure and orientation.
- Screenshots now show a real storm-damage assignment instead of test frames.
- Example outlet names removed from the export preset hint and test fixtures.
- Press Tab to show or hide the left sidebar, in the catalog and in Preview (in Preview it only lasts for that visit), like Photo Mechanic. Tab still moves between fields while you type or when a dialog is open. The menu keeps ⌃⌘S as its shortcut, and the toolbar tooltip now mentions Tab.

### Fixed
- Preview's info box in the top-left corner has even padding again: its width was estimated from the number of characters, which came out too narrow, so the end of the second line (e.g. the "x" in "2560 px preview") touched the right edge. The box is now sized from the actual text, with 8 px of room on every side.
- A thin line no longer cuts across the Info panel (and the sidebar) when the filmstrip is hidden. The filmstrip's top border was drawn at a fixed height across the whole window even when the filmstrip wasn't showing; it is now drawn only along the filmstrip itself, and only while it is shown.
- Rating a photo no longer moves it. When the contact sheet was sorted (or filtered) by rating, giving a photo 5 stars sent it to the other end of the sheet and the sheet scrolled after it; in Preview the photo jumped to the end of the order, so the arrow keys had nowhere to go and Preview looked stuck. Now the photo stays where it is and the sheet stays put; choose the sort again (or refresh the folder) to re-sort. Marks are also written to disk in the background, so rating never waits on a slow drive, and anything that touches sidecars (captions, crops, copies, quitting) waits for those writes first.
- Flipping through photos in Preview no longer shows each one small and then jumps it to full size. While the sharp preview loads, the thumbnail (or a smaller preview already in memory) is drawn stretched to exactly the size and place the sharp one will take, upright for portrait frames, in fit, 1:1 and zoomed views alike, so the swap is seamless. Apature also loads the next two or three photos in the direction you are going (and one behind) ahead of time, keeps a handful in memory, and drops queued work for photos you have already flipped past, so the photo you land on always loads first.
- Single-key shortcuts now work on the Mac: T tags, X rejects, C crops, ⌥C removes a crop, V compares, Z zooms, B brightens, L loupes, S opens a burst and the bare number keys rate (or set the class, per Settings), in both layouts. Before, macOS only passed ⌘ and ⌃ shortcuts to the menus, so these showed in the menus but did nothing. Each press acts exactly once, and nothing fires while you are typing in a caption, the rename box or a dialog field.
- Marking never changes your JPEG files any more. Ratings, colour classes, tags, rejects, captions and the assignment pad for a JPEG now go to an XMP sidecar beside it (`name.xmp`, or the RAW's sidecar for a RAW+JPEG pair), the same place RAW marks already go. Marks earlier versions wrote inside JPEGs still show; a newer mark in the sidecar wins. If you want marks written into the JPEG too (Photo Mechanic's habit), turn on Settings › General › Write marks into JPEG files; it is off for everyone, including existing users, and when on it now keeps the file's created and modified dates.
- Clearing a caption, headline, keywords or any other caption field on a JPEG now sticks. With marks kept in sidecars, a cleared field used to come back from the text embedded in the JPEG the next time the folder opened; the sidecar now records the clear, and the JPEG itself is still not touched. Photos whose JPEG carries older marks than the sidecar no longer show a conflict marker either: the sidecar is the record (conflicts are still reported if you turn on Write marks into JPEG files).
- A cancelled Copy Photos shows how far it got instead of 100 %, and Compare's status names the right key to stop it (V in the Photo Mechanic layout).
- Canon CR3 files now show their capture date, camera and exposure, so Copy Photos files them in the right dated folder and ingest names using the date work for CR3 too (before, CR3 fell back to the file's date).
- The folder tree no longer shows a drive opened with nothing under it. If a drive (or the empty folder macOS keeps for it) had been looked at before the drive finished mounting, that empty or failed listing used to stick for as long as Apature ran, because a favorite on that drive already counted as being there. Apature now notices when a drive actually mounts and reads it again. A favorite on a drive that isn't plugged in says "Not connected", and an open folder says "Loading…", "No subfolders" or "Can't be read" instead of showing an open triangle with nothing below it.
- The Favorites hint ("Add the open folder with File › Add Folder to Favorites") only shows while you have no favorites.
- Opening a folder that is already open in a tab, by another path to it (a link, or Macintosh HD › Volumes › your drive), switches to that tab instead of opening a second one. If a folder failed to open while restoring tabs, the tab you were on keeps its photos instead of turning blank.
- A preview that finishes loading after you have moved on can no longer be shown in place of another photo: each result has to match the file it was made for, as well as the folder and tab. Preview also keeps the photos either side of the one you're looking at (in the sort order you see), so flipping back and forth reloads less.
- The Zoom section now shows when Preview opens in its own window. That window had no right-hand panel, so Info and Zoom were missing there. Preview also drops the small thumbnail at the top of the panel, so Zoom sits right under Info without scrolling.
- Tab now shows and hides the sidebar in the separate Preview window too. That window never received the key and had no sidebar to show; now it has both, and Tab in the main window still toggles the catalog's sidebar.
- After pressing Tab in Preview, the arrow keys change photos again straight away; before, a hidden control took the keyboard and you had to click the photo first. Tab now also shows and hides the right-hand panel (Info, Zoom, Marks) together with the sidebar, in Preview and in the catalog, as in Photo Mechanic: if either panel is showing, Tab hides both, otherwise it shows both. Preview keeps that for the session; the catalog saves it.
- Exporting or extracting a photo whose caption, keywords or other fields are too long for one JPEG metadata block (about 64 KB) now fails with a clear message instead of writing a broken JPEG and reporting success. Normal-length captions are unaffected.
- SFTP delivery now checks that it is talking to the real server before sending your password or key: the server's key must match the one in ~/.ssh/known_hosts (the file Terminal's ssh uses). An unknown server is refused with instructions to connect once in Terminal and confirm it; a changed key is refused with a warning and nothing is sent. FTP, FTPS and folder delivery are unchanged.
- Exporting several RAW files at once with "Render RAW with the macOS engine" could mix up or lose pictures when two RAWs shared a file name (for example a/IMG_0001.CR2 and b/IMG_0001.NEF), because both used the same temporary file. Each render now gets its own.

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
