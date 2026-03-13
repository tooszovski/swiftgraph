#!/bin/bash
set -euo pipefail

# Release script for SwiftGraph
# Usage: ./scripts/release.sh [version]
# Example: ./scripts/release.sh 0.5.0

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')}"
TAG="v${VERSION}"

echo "==> Releasing SwiftGraph ${TAG}"

# 1. Verify clean working tree
if [ -n "$(git status --porcelain)" ]; then
  echo "ERROR: Working tree is dirty. Commit changes first."
  exit 1
fi

# 2. Create and push tag
echo "==> Creating tag ${TAG}"
git tag -a "${TAG}" -m "Release ${TAG}"
git push origin main --tags

# 3. Create GitHub release
echo "==> Creating GitHub release"
gh release create "${TAG}" \
  --title "SwiftGraph ${TAG}" \
  --generate-notes

# 4. Download tarball and compute SHA256
echo "==> Computing SHA256 for formula"
TARBALL_URL="https://github.com/tooszovski/swiftgraph/archive/refs/tags/${TAG}.tar.gz"
SHA256=$(curl -sL "${TARBALL_URL}" | shasum -a 256 | cut -d' ' -f1)
echo "SHA256: ${SHA256}"

# 5. Update formula
sed -i '' "s|sha256 \".*\"|sha256 \"${SHA256}\"|" Formula/swiftgraph.rb
sed -i '' "s|/tags/v.*\.tar\.gz|/tags/${TAG}.tar.gz|" Formula/swiftgraph.rb

echo ""
echo "==> Done! Next steps:"
echo "  1. Copy Formula/swiftgraph.rb to your homebrew-tap repo"
echo "  2. git push in the tap repo"
echo "  3. Users can install with:"
echo "     brew tap tooszovski/tap"
echo "     brew install swiftgraph"
