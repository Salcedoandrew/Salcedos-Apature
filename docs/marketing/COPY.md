# Apature copy kit

Approved wording, in Caleb's voice. Use these as written; cut, don't rewrite.
No outlet names, no client names, no promises about features that are not in
`docs/STATUS.md` as implemented.

## Name and pronunciation

**Apature**, one word, capital A. Say it like "aperture". The spelling is the
app's, not a typo.

## One line

- Card in, cull, caption, export, send. The Photo Mechanic loop, rebuilt in Rust for the Mac.
- A keyboard-first contact sheet for photojournalists. Free and open source.
- Ingest, cull, caption, export, deliver. Originals are never modified.

## Taglines (hero size)

- Card in. Cull. Caption. Send.
- Built for the deadline, not the catalogue.
- The loop you already run. Verified.

## Short (app directories, social bios, under 300 characters)

Apature is a free, open-source contact sheet for photojournalists on a Mac.
Ingest a card with every copy hashed, cull with ratings and colour classes,
caption with Photo Mechanic's variables, export JPEG or JPEG XL, and deliver
to a folder or server. Originals are never modified.

## Medium (release post, forum, newsletter)

Apature rebuilds the loop a working photojournalist runs every week, card in,
cull, caption, export, send, as a native Mac app written in Rust. It keeps
what Photo Mechanic gets right: the keys, the variables, the sidecar
metadata that every other tool can read. It adds the things I wanted on
deadline: a hash check on every copy at every destination, day chips before
you ingest, a ledger so a card is never copied twice, and a sent ledger per
photo so you know what reached the desk.

It is free and open source. There are no accounts and no analytics. Ratings,
captions, rotation and crops live in an XMP sidecar next to the file, so a
folder Apature touched still opens anywhere.

This is 0.1.0, the first shareable build. It runs my own assignments end to
end, and it is early. Tell me what breaks and what you would want next.

## Long (about page)

Use the "Why I made this" section of the README verbatim. It is Caleb's own
text and should not be paraphrased.

## Feature bullets (pick six)

- Ingest from a card or any folder, with day chips before you copy and a live example of every file name.
- SHA-256 verification of every copy at every destination, and a ledger so a card is never ingested twice.
- Cull on the sheet with ratings, colour classes, tags, filters, burst stacks, compare up to four frames, a loupe and 1:1.
- Caption with an assignment pad, code replacements, keywords, GPS and the variables Photo Mechanic users already know.
- Export JPEG or JPEG XL with presets per outlet; EXIF, XMP and IPTC-IIM all kept.
- Deliver to a folder, FTP, FTPS or SFTP, passwords in the Keychain, with retry and a per-photo sent ledger.
- Crop and rotate stored in the sidecar. The original is never modified.
- Two appearances, six soft accent colours, Reduce Motion, text scaling, higher contrast, focus rings.
- Live Ingest for a watched folder and update checks against GitHub Releases.
- No accounts, no analytics, no telemetry.

## Social posts

**Launch (short)**
I rebuilt the Photo Mechanic loop as a free Mac app in Rust. Card in, cull,
caption, export, send, with every copy verified and originals never touched.
Apature 0.1.0 is out. Tell me what breaks.

**Launch (thread opener)**
I love Photo Mechanic and I use it on deadline. I also wanted to see what I
could build myself. Apature is the result: a keyboard-first contact sheet
for photojournalists, free and open source, for the Mac. Here's what it does.

**Artists call**
Apature needs an illustrator and an animator. The app has a mascot brief
ready (a fennec named Fen), a design system, and a dark, quiet interface
that could use one warm thing in it. Paid. Details in the README.

## Words to avoid

- "replacement for Photo Mechanic" as a claim. Say "rebuilt the loop" or "the Photo Mechanic loop".
- "AI", "smart", "powered by". The app has none of it and should not imply it.
- "professional-grade", "industry-standard", "blazing fast". Measure or leave it out.
- Any outlet or client name, including in example captions and screenshots.

## Facts you can quote

| Fact | Source |
| --- | --- |
| Version 0.1.0, released 2026-10-21 | CHANGELOG.md |
| macOS 12 and later | README.md |
| Tested on a Nikon Z 6 II and an M1 Pro Mac | README.md |
| Formats: NEF, RAW+JPEG pairs, JPEG; exports JPEG and JPEG XL | docs/STATUS.md |
| Licence: MIT or Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| DMG size about 7 MB | GitHub release v0.1.0 |
