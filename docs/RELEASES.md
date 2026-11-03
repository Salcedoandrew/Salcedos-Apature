# Releases

How work reaches a Mac: branches, the changelog, version numbers, betas and what goes out beside a build. Adapted
from Folio's releases standard (its REL rules), kept to what fits a Rust desktop app with one maintainer. Rule ids
are stable so a pull request or a commit can cite them.

## Rules

- **REL-1 MUST** keep `main` releasable. Every change lands as a topic branch and a squashed pull request, or, for a
  small change, as a direct commit that passed the same checks locally (fmt, clippy, tests, bundle). Direct pushes
  are allowed by the owner's choice; the branch protection only forbids force-pushes and deletion.
- **REL-2 MUST NOT** let a topic branch live long enough to hold a release. Rebase onto `main` whenever it moves.
- **REL-4 MUST NOT** build, tag or publish anything from a topic branch. Releases are cut from `main`.
- **REL-5 MUST** name branches after the change, never after the agent, the session or the tool. No AI attribution
  anywhere: no co-author trailer, no "generated with" footer, no robot credit in a commit, a description or a doc.
  Not waivable. Checked by `tools/check-release-rules.sh`.
- **REL-7 MUST** keep the notes for the next release on `main` in a `## [x.y.z] — Unreleased` section of
  `CHANGELOG.md`, and add each user-visible change's line in the same pull request as the change. Label
  `no-changelog` when nobody will see a difference.
- **REL-9 MUST** write each line as what a person sees, not what the code does, and punctuate without em dashes.
  The rule is about the prose under a heading: the dated heading keeps the em dash REL-10 gives it, and
  `tools/check-release-rules.sh` skips heading lines when it looks for one.
- **REL-10 MUST** date the section in the version-bump commit, not before. `Unreleased` is the truth until then.
- **REL-11 MUST** follow semver in `Cargo.toml` (`[workspace.package] version`). The build number is the commit
  count on `main` and is derived, never typed.
- **REL-13 MUST** make the version bump its own commit, the last one before the tag, touching only `Cargo.toml`,
  `Cargo.lock`, `CHANGELOG.md` and the release doc. Label `release-exception` when a release and a change really do
  belong together.
- **REL-16** A version number means one build. If what ships has to change, the number moves.
- **REL-17 MUST** cut betas from `main`, tagged `vX.Y.Z-beta.N`, published as GitHub pre-releases.
- **REL-18 MUST** sign every build, beta included, with the Developer ID once it is in use; until then every
  release says it is ad-hoc signed and how to open it.
- **REL-22 MUST** publish the DMG's SHA-256 next to the download, in the release notes.
- **REL-25 MUST** use the author's own words for anything users read: release notes, the README intro, replies.
- **REL-26 SHOULD** keep a short runbook per release in `docs/releases/`, written before the build.
- **REL-30 MUST** take a published screenshot from the build being released (the headless harness does this).
- **REL-33 MUST** keep the in-app updater pointed at GitHub Releases and able to see every release: a stable is a
  normal release, a beta is a pre-release, and the DMG is attached under its `Apature-x.y.z.dmg` name.
- **REL-32 MUST** keep the changelog's top version equal to `Cargo.toml`'s version; CI checks it on every push.

## Cutting a release

1. `main` green, `CHANGELOG.md` Unreleased section complete and in the author's words.
2. Version-bump commit: set the version in `Cargo.toml`, date the changelog section, add `docs/releases/x.y.z.md`.
3. `git tag -a vX.Y.Z -m "Apature X.Y.Z"` and push the tag. The release workflow builds the DMG on a macOS runner,
   signs and notarises when the secrets exist, computes the SHA-256, and attaches both to a GitHub release with the
   changelog section as notes.
4. Download the DMG from the release page and launch it once on a Mac that did not build it.

## Where Apature is today (2026-10-21)

- 0.1.0 tagged and published, ad-hoc signed; `release.yml` on `v*` tags; `check-release-rules.sh` and the version
  check run in CI.
- Repository: squash merges only, branches deleted on merge, `main` cannot be force-pushed or deleted.
- Not yet: Developer ID signing (REL-18), a beta before 0.1.0 (REL-17 starts with 0.2.0), release runbooks.
