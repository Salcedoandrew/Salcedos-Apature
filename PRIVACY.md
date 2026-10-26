# Data and privacy

Apature has no account, no analytics, no crash reporting and no automatic network use. Everything it stores is on
your Mac, and the only network connections it makes are to the delivery targets you set up yourself.

## What is stored, and where

- **Marks, captions, rotation and crop** go into an XMP sidecar next to each photo. Originals are never modified.
- **Settings** (`prefs.json`), the **ingest ledger** of file fingerprints, the **delivery ledger** of what was sent
  where, and the **log** (`apature.log`) live in `~/Library/Application Support/dev.mccal.Apature/`. Delete the
  folder to reset everything.
- **Thumbnails** are cached in the app's cache folder and can be cleared from Settings › Cache & diagnostics.
- **Delivery passwords** are stored in the macOS Keychain under `dev.mccal.Apature.delivery`, one entry per target.

## What leaves the Mac

Once a day, when "Check for updates" is on (it is by default, and Settings › General turns it off), Apature asks
GitHub's public releases API for Salcedo's Apature repository (Salcedoandrew/salcedos-apature) for the newest version. The request
carries the app's own version in its user agent and nothing else; GitHub sees your IP address like any web request.

Only when you click **Update** on that notice does Apature download the release's disk image from GitHub. Nothing is
sent with the download beyond the same user agent. The image is opened, the app inside it is copied out, the copy is
checked to be Apature, and the app is then replaced on your Mac with it; the download and the old app are deleted
afterwards. Settings and photos are not part of it.

Only the files you export, to the targets you configure, over the protocol you chose. The log records the file
names and hosts of those sends; it is never uploaded. GPS you enter is written into sidecars and into exports like
any other field, so check it before delivering if a location should stay private.
