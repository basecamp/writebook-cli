#!/usr/bin/env bash
# Usage: scripts/release.sh VERSION [--dry-run]
#   VERSION: semver, with or without the v prefix (0.2.0, v0.2.0, 0.2.0-rc.1)
#
# A release is two runs. The tag must point at a commit whose Cargo.toml has
# the tag's version, and the version change goes through a pull request. The
# first run sets the version on release/vX.Y.Z, opens a PR and stops. Once the
# PR is merged, the second run from main pushes the tag, and the tag starts the
# release workflow. The script never pushes to main.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
RESET='\033[0m'

info()  { echo -e "${GREEN}==>${RESET} ${BOLD}$*${RESET}"; }
die()   { echo -e "${RED}ERROR:${RESET} $*" >&2; exit 1; }

usage() {
  echo "Usage: scripts/release.sh VERSION [--dry-run]"
}

if [[ $# -eq 0 || $# -gt 2 ]]; then
  usage
  exit 1
fi

VERSION="${1#v}"
DRY_RUN=0
case "${2:-}" in
  "") ;;
  --dry-run) DRY_RUN=1 ;;
  *) die "Unknown argument: '$2' (expected --dry-run)" ;;
esac

# SemVer: no leading zeros in numbers (bash arithmetic would also read 08 as
# octal in the version comparison below), and a prerelease suffix of
# dot-separated identifiers made of letters, digits and hyphens.
NUMBER='(0|[1-9][0-9]*)'
IDENTIFIER='(0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*)'
if [[ ! "$VERSION" =~ ^${NUMBER}\.${NUMBER}\.${NUMBER}(-${IDENTIFIER}(\.${IDENTIFIER})*)?$ ]]; then
  die "Invalid version '${VERSION}' (expected SemVer like 0.2.0 or 0.2.0-rc.1, optionally v-prefixed)"
fi

TAG="v${VERSION}"
PRERELEASE=0
if [[ "$VERSION" == *-* ]]; then
  PRERELEASE=1
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  info "Dry run: no branches, PRs or tags"
  echo ""
fi

DEFAULT_BRANCH=$(git remote show origin 2>/dev/null | sed -n 's/.*HEAD branch: //p')
DEFAULT_BRANCH="${DEFAULT_BRANCH:-main}"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "$DEFAULT_BRANCH" ]]; then
  die "Not on $DEFAULT_BRANCH (currently on $BRANCH)"
fi

if [[ -n "$(git status --porcelain)" ]]; then
  die "The working tree has changes. Commit or stash them first."
fi

# From here on the tree is clean, so a failed run can undo a half-made version
# change by checking the files out of HEAD, and go back to the default branch
# so the next attempt starts there.
restore() {
  if [[ "$1" -ne 0 ]]; then
    git checkout --quiet HEAD -- Cargo.toml Cargo.lock || true
    if [[ "$(git rev-parse --abbrev-ref HEAD)" != "$DEFAULT_BRANCH" ]]; then
      git switch --quiet "$DEFAULT_BRANCH" || true
    fi
  fi
}
trap 'restore "$?"' EXIT

git fetch origin "$DEFAULT_BRANCH" --quiet
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse "origin/$DEFAULT_BRANCH")
if [[ "$LOCAL" != "$REMOTE" ]]; then
  die "Local $DEFAULT_BRANCH (${LOCAL:0:7}) is not the same as origin (${REMOTE:0:7}). Pull or push first."
fi

cargo_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1
}

# Refuse a release that can't happen before anything is pushed.
git fetch origin --tags --quiet
if git rev-parse -q --verify "refs/tags/${TAG}^{commit}" >/dev/null; then
  EXISTING_SHA=$(git rev-parse "refs/tags/${TAG}^{commit}")
  if [[ "$EXISTING_SHA" != "$LOCAL" ]]; then
    die "Tag $TAG already exists at ${EXISTING_SHA:0:7}, not at HEAD. Choose a new version."
  fi
  if [[ "$(cargo_version)" != "$VERSION" ]]; then
    die "Tag $TAG already exists at HEAD, but Cargo.toml says $(cargo_version). Choose a new version."
  fi
fi

version_lt() {
  local -a a b
  local i
  IFS=. read -r -a a <<< "${1#v}"
  IFS=. read -r -a b <<< "${2#v}"
  for i in 0 1 2; do
    if (( 10#${a[i]:-0} < 10#${b[i]:-0} )); then return 0; fi
    if (( 10#${a[i]:-0} > 10#${b[i]:-0} )); then return 1; fi
  done
  return 1
}

# A prerelease comes before its version, so 0.2.0-rc.1 must be newer than
# 0.1.x but is older than 0.2.0.
LATEST_STABLE=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-version:refname | awk '!/-/ { print; exit }')
if [[ -n "$LATEST_STABLE" && "$LATEST_STABLE" != "$TAG" ]]; then
  if [[ "$PRERELEASE" -eq 1 ]]; then
    if ! version_lt "$LATEST_STABLE" "${VERSION%%-*}"; then
      die "Version $VERSION is older than the latest release ${LATEST_STABLE#v}."
    fi
  elif version_lt "$TAG" "$LATEST_STABLE"; then
    die "Version $VERSION is older than the latest release ${LATEST_STABLE#v}."
  fi
fi

info "Running release checks"
info "  Branch: $BRANCH"
info "  Commit: ${LOCAL:0:7}"
info "  Tag:    $TAG"
echo ""
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --bins --test read --test login --test instances --test upgrade

next_steps() {
  echo ""
  echo "  1. Merge $1"
  echo "  2. Pull $DEFAULT_BRANCH and run: scripts/release.sh $VERSION"
  echo "     It finds the version set and pushes the tag."
}

open_release_pr() {
  local branch="release/${TAG}" existing url
  command -v gh >/dev/null 2>&1 || die "gh is needed to open the release PR (https://cli.github.com)."

  existing=$(gh pr list --head "$branch" --base "$DEFAULT_BRANCH" --state open --json url --jq '.[0].url // empty')
  if [[ -n "$existing" ]]; then
    info "The release PR for $TAG is already open: $existing"
    next_steps "$existing"
    return
  fi

  # A release branch here or on origin with no open PR is an earlier attempt.
  # Resetting or pushing over it could throw away work.
  if git rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
    die "$branch already exists locally. Delete it (git branch -D $branch) and run this again."
  fi
  if [[ -n "$(git ls-remote --heads origin "$branch")" ]]; then
    die "origin already has $branch but no open PR from it. Open a PR from it, or delete it (git push origin --delete $branch) and run this again."
  fi

  info "Setting the version to $VERSION on $branch"
  git switch --quiet -c "$branch"
  perl -0pi -e "s/^version = \".*\"/version = \"${VERSION}\"/m" Cargo.toml
  cargo update --workspace --offline --quiet
  git add Cargo.toml Cargo.lock
  git commit --quiet -m "Set the version to ${VERSION}"
  git push --quiet --set-upstream origin "$branch"

  url=$(gh pr create --base "$DEFAULT_BRANCH" --head "$branch" --label release \
    --title "Prepare ${TAG} release" \
    --body "Sets the version in \`Cargo.toml\` to ${VERSION}.

Opened by \`scripts/release.sh\`. Once this is merged, run \`scripts/release.sh ${VERSION}\` again from an up-to-date \`${DEFAULT_BRANCH}\` to push the ${TAG} tag.")
  git switch --quiet "$DEFAULT_BRANCH"
  info "Opened the release PR: $url"
  next_steps "$url"
}

if [[ "$(cargo_version)" != "$VERSION" ]]; then
  if [[ "$DRY_RUN" -eq 1 ]]; then
    info "Cargo.toml says $(cargo_version), not $VERSION"
    echo "  A real run sets the version on release/${TAG}, opens the release PR and stops."
    echo "  The run after that PR is merged pushes the tag."
    echo ""
    info "Dry run complete. No branch, PR or tag created."
    exit 0
  fi
  open_release_pr
  exit 0
fi

info "Cargo.toml says $VERSION"

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo ""
  info "Dry run complete. No tag created."
  exit 0
fi

if git rev-parse -q --verify "refs/tags/${TAG}^{commit}" >/dev/null; then
  info "Tag $TAG already exists at HEAD"
else
  info "Creating tag $TAG"
  git tag -a "$TAG" -m "Release $TAG"
fi

# Someone else can push the same tag while the checks run. On any rejection
# the local tag is removed so the next attempt starts clean.
info "Pushing $TAG to origin"
if ! push_output=$(git push origin "refs/tags/${TAG}" 2>&1); then
  echo "$push_output" >&2
  git tag -d "$TAG" >/dev/null
  git fetch origin --tags --quiet || true
  die "The push of $TAG was rejected (above). Nothing was pushed and the local tag was removed. If $TAG now exists on origin, choose a new version."
fi

echo ""
info "Release $TAG started"
echo ""
echo "  Actions: https://github.com/basecamp/writebook-cli/actions"
echo "  Release: https://github.com/basecamp/writebook-cli/releases/tag/$TAG"
