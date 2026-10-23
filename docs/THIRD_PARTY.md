# Third-party dependencies and licence obligations

Generated 2026-10-13 from `cargo metadata` (452 third-party packages in the
full graph, most of them pulled in by eframe). Re-run `cargo metadata --format-version 1 | python3 -I scripts/licenses.py`
whenever dependencies change.

## Direct dependencies

| crate | version | licence | why |
|---|---|---|---|
| eframe / egui | 0.33 | MIT OR Apache-2.0 | desktop UI (pinned: 0.36 needs rustc 1.95) |
| jpeg-decoder | 0.3 | MIT OR Apache-2.0 | JPEG decode with DCT-domain scaling |
| image | 0.25 (jpeg only) | MIT OR Apache-2.0 | rotate, box-filter thumbnail, JPEG encode for cache |
| memmap2 | 0.9 | MIT OR Apache-2.0 | read-only mapping of RAW files |
| regex | 1 | MIT OR Apache-2.0 | targeted XMP property edits |
| rayon | 1 | MIT OR Apache-2.0 | parallel sidecar reads, CLI benchmark |
| crossbeam-channel | 0.5 | MIT OR Apache-2.0 | worker → UI results |
| rfd | 0.17 | MIT | native NSOpenPanel folder picker |
| directories | 6 | MIT OR Apache-2.0 | cache location |
| walkdir | 2 | Unlicense OR MIT | folder scan |
| trash | 5 | MIT | move files to the system Trash (NSFileManager on macOS) |
| sha2 | 0.10 | MIT OR Apache-2.0 | ingest verification |
| clap, anyhow, thiserror, serde, log, env_logger | current | MIT OR Apache-2.0 | plumbing |

## Transitive packages that are not plain MIT/Apache

| licence | packages | obligation for us |
|---|---|---|
| MPL-2.0 | `option-ext` (via `directories`) | file-level copyleft; none unless we modify its source |
| MIT OR Apache-2.0 OR LGPL-2.1-or-later | `r-efi` (via `getrandom`) | tri-licensed; we take MIT |
| (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | `epaint_default_fonts` | the bundled Ubuntu and Noto fonts need their licence notices shipped with binaries |
| Unicode-3.0 | ICU data crates (via egui text shaping) | keep the notice |
| Zlib, BSL-1.0, BSD-2/3-Clause, ISC, CC0-1.0, 0BSD | assorted | attribution only |

No GPL, AGPL or CDDL code is in the graph. **LibRaw (LGPL-2.1/CDDL) and
rawler (LGPL-2.1) are deliberately not used**; embedded-preview extraction is
implemented in `apature-core::tiff` + `apature-preview::embedded` under our own
licence. If a RAW *decode* fallback is ever added, that decision must revisit
this file (see `docs/DECISIONS.md`, ADR-002).

## What we must ship with a binary

A `THIRD_PARTY_NOTICES` file containing the licence texts of every package
above, including the font licences. Not yet generated; blocked on choosing a
tool (`cargo-about` is the usual answer). Tracked in `docs/STATUS.md`.

## Delivery (added 2026-10-19)
- suppaftp 6 (Apache-2.0 / MIT) — FTP and FTPS client.
- ssh2 0.9 (MIT / Apache-2.0), libssh2 (BSD-3-Clause) and OpenSSL (Apache-2.0, vendored) — SFTP.
- security-framework 3 (MIT / Apache-2.0) — macOS Keychain for delivery passwords.
- cjxl (libjxl, BSD-3-Clause) — optional external tool for JPEG XL export; not bundled, found via Homebrew/PATH/APATURE_CJXL.
- tray-icon 0.26 (MIT / Apache-2.0) — the macOS menu bar extra.
- Ported code (not a crate dependency): RAW preview rules from radenkovic/extract-raw-preview (MIT), see `THIRD_PARTY_NOTICES.md`.
