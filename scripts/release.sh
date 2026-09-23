#!/bin/bash
set -euo pipefail

# Release script for SwiftGraph.
#
# Usage: ./scripts/release.sh X.Y.Z
#
# Safe to re-run: every step checks whether it is already done.
#   1. verify: on main, clean tree, Cargo.toml version == X.Y.Z, in sync with origin
#   2. quality gates (fmt, clippy, tests) BEFORE anything is tagged or pushed
#   3. annotated tag vX.Y.Z on HEAD (reused if it already points at HEAD)
#   4. push main and the tag
#
# Everything after the tag is done by .github/workflows/release.yml:
# GitHub release, binary asset, and the formula bump in tooszovski/homebrew-tap.
#
# Why the formula is not bumped here: the formula pins the SHA256 of the tag's
# source tarball, which only exists once the tag does. Writing it into the
# tagged commit is impossible (the tarball would change), so the single source
# of truth for the formula is the tap repo, updated by the workflow.

usage() {
  echo "Usage: $0 X.Y.Z" >&2
  exit 2
}

[ $# -eq 1 ] || usage
VERSION="$1"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
TAG="v${VERSION}"

cd "$(git rev-parse --show-toplevel)"
echo "==> Releasing SwiftGraph ${TAG}"

# 1. Verify state
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$BRANCH" != "main" ]; then
  echo "ERROR: releases are cut from main (current: ${BRANCH})." >&2
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "ERROR: working tree is dirty. Commit changes first." >&2
  exit 1
fi
CARGO_VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  echo "ERROR: Cargo.toml version is ${CARGO_VERSION}, expected ${VERSION}." >&2
  echo "       Bump [workspace.package] version, run cargo check to refresh Cargo.lock, commit." >&2
  exit 1
fi
git fetch --quiet origin main --tags
if ! git merge-base --is-ancestor origin/main HEAD; then
  echo "ERROR: HEAD is behind origin/main. Pull first." >&2
  exit 1
fi

HEAD_SHA="$(git rev-parse HEAD)"
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
  TAG_SHA="$(git rev-parse "${TAG}^{commit}")"
  if [ "$TAG_SHA" != "$HEAD_SHA" ]; then
    echo "ERROR: tag ${TAG} exists and points at ${TAG_SHA}, not HEAD (${HEAD_SHA})." >&2
    exit 1
  fi
  TAG_EXISTS=1
else
  TAG_EXISTS=0
fi

# 2. Quality gates (skipped when the tag already exists: it was verified then)
if [ "$TAG_EXISTS" -eq 0 ]; then
  echo "==> Running quality gates"
  cargo fmt --all -- --check
  cargo clippy --workspace --locked -- -D warnings
  cargo test --workspace --locked
fi

# 3. Tag
if [ "$TAG_EXISTS" -eq 0 ]; then
  echo "==> Creating tag ${TAG}"
  git tag -a "${TAG}" -m "Release ${TAG}"
else
  echo "==> Tag ${TAG} already on HEAD, reusing it"
fi

# 4. Push (no-op when already pushed)
echo "==> Pushing main and ${TAG}"
git push origin main
git push origin "refs/tags/${TAG}"

echo ""
echo "==> Tag pushed. The release workflow now builds and publishes ${TAG}:"
echo "    https://github.com/tooszovski/swiftgraph/actions/workflows/release.yml"
echo "    Users install with:"
echo "      brew tap tooszovski/tap"
echo "      brew install swiftgraph"
