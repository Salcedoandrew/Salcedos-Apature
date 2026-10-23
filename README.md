## About this fork

Based on **Apature** by [McCal-Codes](https://github.com/McCal-Codes/apature), the original project by Caleb McCartney.
This edition is by [Andrew Salcedo](https://github.com/Salcedoandrew/salcedos-apature) and adds:

- Photo Mechanic keyboard shortcuts
- RAW preview extraction for all major cameras
- Select By rating, color class and tag
- Copy Photos
- Extract JPEG Previews from RAW
- Sidebar folder tree
- G closes Preview
- Ratings saved to XMP sidecars (originals untouched)
- Rating and preview fixes
- Help › Report a Bug

Test builds are on [Salcedo's Apature releases](https://github.com/Salcedoandrew/salcedos-apature/releases); report problems in
[its Issues](https://github.com/Salcedoandrew/salcedos-apature/issues). The original README follows; the licence is unchanged.

### AI disclosure

The changes in this edition were written with the help of AI coding assistants. Andrew Salcedo directed the work and tested every change, but some bugs may remain, so please report any problems through Help › Report a Bug… or the Issues tab. The original Apature is by McCal-Codes.

---

<p align="center">
  <img src="docs/images/apature-icon.png" width="96" alt="Apature app icon">
</p>

<h1 align="center">Apature</h1>

<p align="center">
  <b>Apature: a keyboard-first contact sheet, culling, captions and delivery for photojournalists</b><br>
  the Photo Mechanic loop, rebuilt in Rust for the Mac, free and open source.
</p>

<p align="center">
  <a href="https://github.com/McCal-Codes/apature/releases/latest"><img src="https://img.shields.io/github/v/release/McCal-Codes/apature?label=download" alt="Download the latest release"></a>
  <a href="https://github.com/McCal-Codes/apature/actions/workflows/ci.yml"><img src="https://github.com/McCal-Codes/apature/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20or%20Apache--2.0-blue" alt="License: MIT or Apache-2.0"></a>
  <img src="https://img.shields.io/badge/macOS-12%2B-000000?logo=apple" alt="macOS 12+">
  <img src="https://img.shields.io/badge/Rust-stable-DEA584?logo=rust" alt="Rust stable">
  <img src="https://img.shields.io/badge/originals-never%20modified-5A48C7" alt="Originals never modified">
  <a href="https://ko-fi.com/mccal"><img src="https://img.shields.io/badge/Ko--fi-buy%20me%20a%20coffee-FF5E5B" alt="Buy me a coffee on Ko-fi"></a>
</p>

<p align="center">
  <img src="docs/marketing/hero-dark.png" width="820" alt="Apature: card in, cull, caption, send. The contact sheet in the dark appearance with a storm-damage assignment open">
</p>

<div align="center">

| Version | Price | macOS | Camera files | Tested on |
| :---: | :---: | :---: | :---: | :---: |
| 0.1.0 | Free | 12+ | NEF, RAW+JPEG pairs, JPEG | Nikon Z 6 II, M1 Pro |

</div>

> **Status:** 0.1.0, the first shareable build. It runs my own assignments end to end, but it is early: expect rough
> edges, and tell me about them. Developed on a Mac with a Nikon Z 6 II. See what's changed in [CHANGELOG.md](CHANGELOG.md).

**Contents:** [Why I made this](#why-i-made-this) · [Screenshots](#screenshots) · [What it does](#what-it-does) ·
[What you need](#what-you-need) · [Install](#install) · [Keys](#keys) · [Promises](#promises) · [Get involved](#get-involved) ·
[Artists wanted](#artists-and-animators-wanted) · [Contact](#contact) · [License](#license)

## Why I made this

Honestly, I made this to see what I could do. I love Photo Mechanic and I use it on deadline, but I was curious what
I could build myself, so I rebuilt the loop I actually use every week: card in, cull, caption, export, send. Along
the way I added a few things it doesn't have, like hashed verification of every copy, day chips before you ingest,
and delivery with a sent ledger, and I kept the things it gets right, like the keys and the variables.

Where I want this to go is onboarding. Getting into photojournalism shouldn't mean a cold start with software that
assumes you already know the workflow. I plan for Apature to teach the loop as you use it, the same loop you would
run in Photo Mechanic, so that someone picking it up for the first time can learn the job and the tool together. I
don't like gatekeeping. I'd like the people already in the industry to see this as an effort to bring more people
in, and to show that it isn't that hard. Sometimes it's the software and the experience, not the person.

Who knows where it goes from here. This is the first version, I'm still learning Rust, and it shows in places. Don't
go crazy about it; tell me what breaks and what you'd want next.

## Screenshots

| Preview with the crop tool | Caption workspace |
| :---: | :---: |
| ![Crop](docs/screenshots/crop.png) | ![Caption](docs/screenshots/caption.png) |

| Ingest from a card | Export and delivery |
| :---: | :---: |
| ![Ingest](docs/screenshots/ingest.png) | ![Export](docs/screenshots/export.png) |

## What it does

- **Ingest** from a card or any folder: day chips before you copy, a live example of every name the templates produce,
  SHA-256 verification of every copy at every destination, a ledger so a card is never copied twice, voice-memo and
  sidecar companions, a backup destination, eject when verified, and Live Ingest for a watched folder.
- **Cull** on the sheet: ratings, colour classes, tags, filters, sort, tabs, burst stacks, compare up to four frames, a
  loupe, 1:1 with one shared magnification, a view-only exposure boost, rotate and crop stored in the sidecar.
- **Caption** with an assignment pad, code replacements, Photo Mechanic's variables, keywords, GPS, recent values and
  save-to-selection.
- **Copy** the selection to another folder with Copy Photos… (⌘Y): rename on the way, bring partners and sidecars, dated
  subfolders, skip / number / replace on a name clash, byte-for-byte with the original dates. Edit › Select Tagged, by
  Rating, by Color Class, Cropped and RAW+JPEG pairs pick from the current tab.
- **Export** JPEG or JPEG XL with presets per outlet, EXIF, XMP and IPTC-IIM kept, then **deliver** to a folder, FTP,
  FTPS or SFTP with passwords in the Keychain, retry, and a per-photo sent ledger.
- Two appearances, Reduce Motion, text scaling, higher contrast, focus rings, a menu bar extra with live status.

## What you need

- A Mac on macOS 12 or newer. Apple silicon and Intel both build; Apple silicon is what it is tested on.
- Photos from a camera that embeds a JPEG preview in its RAW files, which is every current body. Apature never
  decodes a RAW to show a thumbnail when the camera already did the work.
- Optional: `brew install jpeg-xl` for JPEG XL export. Nothing else.

## RAW previews

Apature shows the JPEG the camera embedded in the RAW and picks the largest
one by default. Checked against one sample per camera from
[raw.pixls.us](https://raw.pixls.us) (`tests/raw-samples/fetch.sh`). The size
is the biggest preview in that sample file; other bodies of the same format can
carry bigger or smaller ones.

| Format | Cameras | Status | Preview found in sample |
|---|---|---|---|
| CR2 | Canon (EOS 40D) | ✅ | 1936×1288 |
| CR3 | Canon (EOS R6) | ✅ | 3408×2272 |
| CRW | Canon (EOS 10D) | ✅ | 1536×1024 |
| NEF | Nikon (D70s) | ✅ | 3008×2000 |
| NRW | Nikon (Coolpix P7000) | ✅ | 3648×2736 |
| ARW | Sony (α7S) | ✅ | 1616×1080 |
| SRF | Sony (DSC-F828) | ✅ | 640×480, the largest the file holds |
| SR2 | Sony (DSC-R1) | ✅ | 640×424, the largest the file holds |
| RAF | Fujifilm (X-T3; FinePix S5000) | ✅ | 4416×2944; 1280×960 |
| RW2 | Panasonic (DMC-LX7) | ✅ | 1920×1920 (shot 1:1) |
| RWL | Leica (D-LUX 4) | ✅ | 1920×1280 |
| ORF | Olympus (E-M10; E-10) | ✅ | 3200×2400; 160×120, the largest the E-10 file holds |
| PEF | Pentax (K10D) | ✅ | 3872×2592 |
| DNG | Adobe DNG Converter, Pentax K-x, Apple ProRAW (iPhone 12 Pro) | ✅ | 3960×2640, 4288×2848, 4032×3024 |
| 3FR | Hasselblad (CFV; X1D) | ⚠️ no JPEG in file, RGB thumbnail re-encoded | 218×218; 1440×1080 |
| FFF | Hasselblad (H5D-40; X1D II) | ⚠️ no JPEG in file, RGB thumbnail re-encoded | 1217×913; 1181×885 |
| IIQ | Phase One (P25+; IQ150) | ⚠️ no JPEG in file, RGB thumbnail re-encoded | 300×220; 588×442 |
| SRW | Samsung (NX100) | ✅ | 4592×3056 |
| X3F | Sigma (DP1) | ✅ | 2640×1760 |

`apature-cli preview list FILE` shows every preview in a file;
`apature-cli preview extract FILE [--prefer largest|smallest|PIXELS] [--max-bytes N] [--out x.jpg]` writes one out
untouched (default: the largest under 8 MiB). Lossless JPEG streams, which are raw sensor data rather than
previews, are never picked.

## Install

Download the DMG from the [latest release](https://github.com/McCal-Codes/apature/releases/latest) and drag Apature
to Applications. 0.1.0 is not yet notarised: the first time, right-click the app and choose Open, or allow it under
System Settings › Privacy & Security.

Build it yourself with Rust 1.80 or newer:

```bash
scripts/bundle.sh --install   # builds, bundles, signs ad-hoc, installs to /Applications
```

## Keys

Apature uses Photo Mechanic's macOS shortcuts out of the box, so your hands already know the way. Prefer Apature's
original keys? Pick **Apature classic** in Settings › Shortcuts › Shortcut layout; the choice is remembered. The full
table is also in Settings › Shortcuts.

| Action | Photo Mechanic (default) | Apature classic |
| --- | --- | --- |
| Colour class 1–8 · clear | `⌘1`–`⌘8` · `⌘0` | `1`–`8` · `0` |
| Star rating 1–5 · clear | `⌃1`–`⌃5` · `⌃0` | `⌘1`–`⌘5` · `⌘0` |
| Bare number keys | stars (default) or colour class, per Settings › Shortcuts › Number keys | colour class (the Number keys setting applies off the Mac) |
| Toggle tag | `T` | `T` |
| Reject | `X` | `X` |
| Preview · back | `Space` or `⏎` · `Esc` | `Space` or `⏎` · `Esc` |
| Preview workspace | `⌘R` | `⌘⇧2` |
| Zoom 1:1 / fit | `Z` | `Z` |
| Compare the selection | `V` | `C` |
| Crop tool · remove crop | `C` · `⌥C` | `K` · `⌫` in the crop tool |
| Rotate left · right | `[` · `]` or `⌘[` · `⌘]` | `⌘[` · `⌘]` |
| Rename photos | `M` or `⌘M` | `⌘⇧R` |
| Copy photos (with options) | `⌘Y` | `⌘Y` |
| Caption the photo (Info) | `I` | `⌘⇧3` |
| Apply assignment pad (metadata template) | `⌘⇧I` | none |
| Copy · paste metadata | `⌥⌘C` · `⌥⌘V` | `⌥⌘C` · `⌥⌘V` |
| Select all · none | `⌘A` · `⌘D` | `⌘A` · `⌘⇧A` |
| Select tagged · cropped | `⌘T` · `⌘⇧C` | none (Edit menu) |
| Select by colour class 1–8 · any · none | `⌘⌃1`–`⌘⌃8` · `⌘⌃9` · `⌘⌃0` | none (Edit menu) |
| Select by rating 1–5 · any · none | `⌥⌘1`–`⌥⌘5` · `⌥⌘6` · `⌥⌘0` | none (Edit menu) |
| Undo · redo | `⌘Z` · `⌘⇧Z` | `⌘Z` · `⌘⇧Z` |
| Move to Trash | `⌘⌫` | `⌘⌫` |
| Ingest · Live Ingest | `⌘G` · `⌘⇧G` | `⌘⇧I` · none |
| Export | `⌃⌘X` | `⌘E` |
| Refresh the folder | `F5` | `⌘R` |
| Reveal in Finder | none | `⌘R` |
| Open folder · new tab · close tab | `⌘O` · `⌘N` · `⌘W` | `⌘O` · `⌘T` · `⌘W` |
| Close window · minimize | `⌘⇧W` · none (`⌘M` renames) | `⌘W` · `⌘M` |
| Next · previous tab | `⌘⇧]` · `⌘⇧[` | `⌘⇧]` · `⌘⇧[` |
| First · last photo | `Home` · `End` | `Home` · `End` |
| Settings | `⌘,` | `⌘,` |

Keys Photo Mechanic has no match for stay the same in both layouts: `L` loupe, `B` brighten, `S` burst stack, `⇧F`
filmstrip, `⌘=` and `⌘-` thumbnail size, `⌃⌘S` sidebar, `⌥⌘I` inspector, `⌥⌘F` clear filters, `⌘⇧1`–`5` workspaces
(Photo Mechanic layout: `⌘R` takes Preview's place), `⌘⏎` save caption and advance.

Not applicable yet (Apature has no matching feature, so these Photo Mechanic keys do nothing): move, and copy from the bare `Y` key,
edit in another app (`E`, `⌘E`), upload and FTP (`U`, `⌘U`, `⌃⌘U`), save as (`S`, `⌘S`), tag on / tag off (`+`, `-`,
`⌘+`, `⌘-`), select rotated (`⌘⇧R`),
invert selection (`⌘⇧O`), view filters `F1`–`F4` and by rating or class (`⌘⇧F1`…, `⌘⌃F1`…), combined RAW+JPEG
(`⌘J`), rescan (`⌘/`), slide show (`⌘L`, `⌘⇧L`), keywords panels (`⌘K`, `⌥⌘K`), metadata template window (`⌘I`),
find and replace (`⌘F`, `⌘⇧F`), print (`⌘P`, `⌘⇧P`), email (`⌘⇧E`), mirror (`⌘⌃[`, `⌘⌃]`), sound (`A`), shadows and
highlights (`N`, `B`), 2-up layouts and linked previews (`O`, `H`, `G`, `/`, `Tab`, `L` in Preview), full screen
(`F`, `R`; use the system's `⌃⌘F`), render quality (`Q`), loupe cursor (`Z` on the sheet), add / remove from
selection in Preview (`W`, `D`), scroll to last previewed (`L`), close other tabs (`⌃⌥⌘W`), new window and ingest from
selection (`⌘⇧N`, `⌘⇧S`), the catalog menu (Photo Mechanic Plus), and the IPTC Info window's own keys.

## Promises

- Originals are never modified. Marks, rotation and crop go to the XMP sidecar next to the file, edited property by
  property so what Photo Mechanic or Lightroom wrote survives.
- `xmp:Rating`, `xmp:Label`, `photomechanic:ColorClass` and `photomechanic:Tagged` are written together, which is
  what both of those need to agree on a label.
- Copies are verified by hash before they count. The card is read-only.
- No account, no analytics, no network use except the delivery targets you set up. See [PRIVACY.md](PRIVACY.md).

Press kit: screenshots, hero images, a social preview and approved wording are in [docs/marketing](docs/marketing).

## Get involved

Bug reports and ideas go to [Issues](https://github.com/McCal-Codes/apature/issues/new/choose); see
[CONTRIBUTING.md](CONTRIBUTING.md) for how changes are made and checked, and [SECURITY.md](SECURITY.md) for anything
sensitive. [docs/STATUS.md](docs/STATUS.md) says what is implemented, tested and verified, with numbers;
[docs/PLAN.md](docs/PLAN.md) is the order of work; [docs/DECISIONS.md](docs/DECISIONS.md) records every choice.

## Artists and animators wanted

Apature has a mascot, Fen the fennec, drawn as a placeholder, and a motion language for the app that is written
down but not yet animated. I'm looking for illustrators and animation artists to work with on Fen, the app icon, and
the marketing: a short reel, stills for the site and the store, and the small one-shot animations the app plays at
earned moments. Start with the [artist brief](docs/design/ARTIST_BRIEF.md), the [brand kit](docs/design/brand/) and
the [motion notes](docs/design/MOTION_DESIGN.md), then open an issue or email me with a few pieces of your work and
we'll talk about scope and terms. Credit is always given; the brief says how the work is licensed.

## Contact

Caleb McCartney · [contact@mcc-cal.com](mailto:contact@mcc-cal.com) · [Ko-fi](https://ko-fi.com/mccal)

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). Third-party
notices are in [docs/THIRD_PARTY.md](docs/THIRD_PARTY.md) and in About Apature. Not affiliated with Camera Bits or
Photo Mechanic.
