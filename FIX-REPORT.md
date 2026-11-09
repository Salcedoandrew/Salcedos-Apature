# FIX-REPORT — Apature v0.1.13

Two defects from `AUDIT.md` are fixed: the updater re-offering the installed
release on stable builds (§1.1), and the ExFAT copy aborting on set-dates
(§2.4), plus the missing flush test seam (§2.2). Nothing else changed: no
refactors, no new dependencies, no installs, no pushes.

## Fix 1 — the updater no longer re-offers the installed stable version

**Root cause.** `scripts/bundle.sh` stamped every build, tagged stable releases
included, with `APATURE_BUILD=$(git rev-list --count HEAD)`, and
`updater::version_for` turned that into `X.Y.Z-test.N`. Semver sorts a
pre-release below its stable, so a v0.1.13 release app called itself
`0.1.13-test.236`, and `pick` saw the stable `v0.1.13` as newer and offered it
again. The same inversion made a stable build accept test releases with
"Include beta releases" off.

**What changed**

- `scripts/bundle.sh` — a tagged commit is now detected (GitHub Actions'
  `GITHUB_REF_TYPE == tag`, or `git describe --exact-match --tags HEAD`) and the
  build is compiled with `APATURE_RELEASE=1`. The commit count is still passed
  as `APATURE_BUILD`, so the About window and bug reports keep the build number;
  the release flag is what keeps it out of the version. Local and ad-hoc builds
  are unchanged: no tag, no flag, `-test.N` stamp as before.
- `apps/desktop/src/updater.rs` — `RELEASE_BUILD` reads `APATURE_RELEASE` at
  compile time, and the new `version_stamped(pkg, build, release)` returns the
  bare `CARGO_PKG_VERSION` for a release no matter what stamp the environment
  supplied. `current_version()` uses it; `APATURE_UPDATE_BUILD` still overrides
  at run time for testing the check without rebuilding.

**Behaviour**

- A released build reports `0.1.13`, so `pick` against `v0.1.13` returns `None`:
  the installed release is never offered again.
- Channels are unchanged: with betas off a stable build sees no test releases;
  with betas on it is offered a newer test build; a prerelease build still sees
  test releases with betas off.
- Local/ad-hoc builds keep identifying as `X.Y.Z-test.N`.

**Tests** (`apps/desktop/src/updater.rs`)

- `a_release_build_stamped_as_test_is_reoffered_its_own_stable_version` — the
  existing characterization test is kept and now asserts the fixed behaviour:
  the release build identifies as `0.1.13` (no `-test`), `pick` returns `None`
  for its own release, no test release with betas off, `0.1.14-test.1` with
  betas on.
- `a_released_build_identifies_as_a_bare_version` (new) — a released build is
  bare `X.Y.Z` for every stamp the environment could supply; a non-release
  build keeps `-test.N`, and a prerelease build still parses as one.
- `a_correctly_stamped_stable_build_is_never_offered_its_own_version` — kept
  unchanged.

## Fix 2 — ExFAT copy: set-dates falls back instead of failing the copy

**Root cause.** `photocopy::copy_preserving` set the original's dates on a copy
that was already written and verified, and propagated any error: on a volume
that refuses `utimensat` with `ENOTSUP` (the same class as `link(2)` and
`F_FULLFSYNC`) a good copy was thrown away.

**What changed**

- `crates/apature-ingest/src/photocopy.rs`
  - `copy_preserving_with(src, dst, replace, set_dates)` takes the date step as
    a `DateSetter` function pointer, the same seam `copy.rs` uses for `link`, so
    the refusal is testable without an exFAT volume.
  - `keep_dates` treats `ENOTSUP`/`EOPNOTSUPP`/`EINVAL`/`ENOTTY` as "this
    filesystem does not do dates": it retries with `copy_mtime` (modified date
    only) and keeps the copy. If even that is refused, the copy is kept without
    dates. Either way the weaker result comes back as a warning that names the
    step and the file, e.g. `keep the original's dates on the copy failed for
    …/A.CR3: Operation not supported (os error 45); kept the copy with its
    modified date only`. Any other errno (`EACCES`, `ENOSPC`, …) still fails as
    `IngestError::Io` naming the step.
  - `copy_preserving` now returns `CopyPreserved { bytes, date_warning }`.
  - `CopyOutcome.warnings` collects those warnings; `run_copy` fills it.
- `apps/desktop/src/copy_job.rs` — `log_outcome` writes each warning with
  `log::warn!`, so a copy kept without its dates is visible in `apature.log`.

**No-overwrite publish is untouched.** `copy.rs` still writes a temp file,
verifies the SHA-256, and publishes with `link(2)` or, when that is refused,
`create_new`-exclusive copy; a destination that appears during the copy is
never replaced (`DestinationExists`), on both the normal and the replace path
of Copy Photos.

**Flush seam** (`crates/apature-ingest/src/copy.rs`)

- `copy_impl_with` now takes `flush: Flusher` (`fn(&File) -> io::Result<()>`)
  beside `link: Linker`; `sync_durable` and `publish_exclusive` thread it
  through, so the `F_FULLFSYNC` → `fsync(2)` fallback can be exercised.

**Tests** (`crates/apature-ingest/src/{copy,photocopy}.rs`)

- `a_refused_full_flush_degrades_to_a_plain_flush_and_keeps_the_copy` (new) — a
  flush returning `ENOTSUP` yields `SyncQuality::Fallback` and a successful,
  verified copy, both with a working hard link and with the exFAT shape
  (no links + no full flush → `Publish::ExclusiveCopy`, `Fallback`).
- `a_flush_that_fails_for_a_real_reason_still_fails_the_copy` (new) — `ENOSPC`
  fails with the step `flush the copy to disk` and publishes nothing.
- `a_destination_that_refuses_dates_keeps_the_copy_with_a_warning` (new) —
  `ENOTSUP` from the date step keeps the copy, sets the modified date, and
  returns a warning naming the step and the file.
- `a_date_failure_that_is_not_a_refusal_still_fails_the_step` (new) — `EACCES`
  still fails, naming `keep the original's dates on the copy`.

## Test counts

Commands (macOS, debug profile, exit status taken from `PIPESTATUS` with
`pipefail`):

```
$ cargo test -p apature-ingest
test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
   Doc-tests apature_ingest
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
INGEST_EXIT=0

$ cargo test -p apature
test result: ok. 107 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
DESKTOP_EXIT=0
```

| Package | Before (AUDIT.md) | After | Added |
| --- | --- | --- | --- |
| `apature-ingest` | 48 passed | 52 passed | 4 |
| `apature` | 106 passed, 1 ignored | 107 passed, 1 ignored | 1 new + 1 rewritten |

- **Tests added:** 5 (4 ingest, 1 updater); 1 existing updater test rewritten
  to assert the fixed behaviour.
- **Total run:** 159 passed, 0 failed, 0 errors, 1 ignored
  (`updater::tests::live_check`, marked `#[ignore = "network"]` — not run, no
  network calls made).

Confirmed by the suite: a stable build reporting bare `0.1.13` is not offered
`0.1.13`; a simulated `ENOTSUP` on set-dates keeps the copy and reports a
warning; a simulated `ENOTSUP` on the full flush yields `SyncQuality::Fallback`
with a successful copy.

## Limits

- No real ExFAT volume was available: both fallbacks are proven against
  injected errnos through the new seams, not on a physical card.
- `scripts/bundle.sh`'s tag detection was syntax-checked (`bash -n`) and its
  two branches reviewed; no DMG was built, since that needs signing and
  packaging steps outside this fix.
