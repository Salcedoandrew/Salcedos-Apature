#!/usr/bin/env bash
# The parts of docs/RELEASES.md a machine can see. Run on a pull request against its base:
#   bash tools/check-release-rules.sh [base-ref]      # default origin/main
# Labels (PR_LABELS, comma separated) waive: no-changelog (REL-7), release-exception (REL-13). REL-5 is not waivable.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
base=${1:-origin/main}
labels=${PR_LABELS:-}
branch=${PR_BRANCH:-$(git rev-parse --abbrev-ref HEAD)}
body=${PR_BODY:-}
fail=0
say() { echo "::error::$1"; fail=1; }

# REL-5: no AI attribution in commits, the description or the branch name.
if git log --format=%B "$base"..HEAD | grep -qiE "co-authored-by:.*(claude|gpt|copilot|anthropic|openai)|generated with \[?(claude|chatgpt|copilot)|🤖 generated"; then
  say "REL-5: a commit carries AI attribution; rewrite the message"
fi
if printf '%s' "$body" | grep -qiE "generated with \[?(claude|chatgpt|copilot)|co-authored-by:.*(claude|gpt)"; then
  say "REL-5: the pull request description carries AI attribution"
fi
if printf '%s' "$branch" | grep -qiE "claude|chatgpt|copilot|codex|cursor|ai/"; then
  say "REL-5: branch '$branch' is named after a tool; name it after the change"
fi

changed=$(git diff --name-only "$base"...HEAD)
ships=$(printf '%s\n' "$changed" | grep -E "^(apps|crates)/" | grep -vE "/tests?/|_test\.rs$" || true)
notes=$(printf '%s\n' "$changed" | grep -x "CHANGELOG.md" || true)
version=$(git diff "$base"...HEAD -- Cargo.toml | grep -E "^[+-]version = " || true)

# REL-7: a change to what ships adds a changelog line.
if [[ -n "$ships" && -z "$notes" ]] && ! printf '%s' "$labels" | grep -q "no-changelog"; then
  say "REL-7: code under apps/ or crates/ changed without a CHANGELOG.md line (label no-changelog if nobody will see a difference)"
fi
# REL-9: no em dashes in new changelog prose. Headings are exempt: REL-10 dates the section as "[x.y.z] — YYYY-MM-DD".
if [[ -n "$notes" ]] && git diff "$base"...HEAD -- CHANGELOG.md | grep -E "^\+" | grep -v "^+++" | grep -vE "^\+#{1,6} " | grep -q "—"; then
  echo "::warning::REL-9: new changelog lines use an em dash; prefer a comma, a colon or a full stop"
fi
# REL-13: the version bump is its own commit.
if [[ -n "$version" ]] && ! printf '%s' "$labels" | grep -q "release-exception"; then
  other=$(printf '%s\n' "$changed" | grep -vE "^(Cargo\.toml|Cargo\.lock|CHANGELOG\.md|docs/releases/.*|docs/RELEASES\.md)$" || true)
  if [[ -n "$other" ]]; then
    say "REL-13: the version bump touches other files: $(printf '%s' "$other" | tr '\n' ' ')"
  fi
fi
exit $fail
