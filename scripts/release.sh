#!/usr/bin/env bash
# ==============================================================================
# Builds Wflow's Linux packages from one version number.
#
# Cargo.toml is the only file that stores the version: build-deb.sh and
# build-run.sh already read it, and build.rs takes it from CARGO_PKG_VERSION.
# This script adds the parts that were easy to get wrong by hand — bumping the
# version, running the packaging, and making the matching commit and tag.
#
#   ./scripts/release.sh                 # package the version in Cargo.toml
#   ./scripts/release.sh --bump patch    # 0.3.10 -> 0.3.11 first
#   ./scripts/release.sh --bump patch --commit --tag
#
# Windows builds use scripts/release.ps1, which fills in installer.iss.
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

BUMP="none"
DO_COMMIT=0
DO_TAG=0
SKIP_BUILD=0

while [ $# -gt 0 ]; do
    case "$1" in
        --bump)
            BUMP="${2:-}"
            shift 2
            ;;
        --bump=*)
            BUMP="${1#*=}"
            shift
            ;;
        --commit) DO_COMMIT=1; shift ;;
        --tag)    DO_TAG=1;  shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        -h|--help)
            sed -n '2,18p' "$0"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 2
            ;;
    esac
done

case "$BUMP" in
    none|patch|minor|major) ;;
    *) echo "--bump must be none, patch, minor or major (got '$BUMP')" >&2; exit 2 ;;
esac

# Only a line starting in column one belongs to [package]; dependency versions
# are indented inside their own tables.
read_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1
}

write_version() {
    awk -v want="$1" '
        !done && /^version *= *"/ { sub(/".*"/, "\"" want "\""); done=1 }
        { print }
    ' Cargo.toml > Cargo.toml.tmp
    mv Cargo.toml.tmp Cargo.toml
}

BEFORE="$(read_version)"
if [ -z "$BEFORE" ]; then
    echo "Could not find a version in Cargo.toml." >&2
    exit 1
fi
echo "Cargo.toml version: v$BEFORE"

VERSION="$BEFORE"
if [ "$BUMP" != "none" ]; then
    IFS=. read -r MAJOR MINOR PATCH <<<"$BEFORE"
    case "$BUMP" in
        major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
        minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
        patch) PATCH=$((PATCH + 1)) ;;
    esac
    VERSION="$MAJOR.$MINOR.$PATCH"
    write_version "$VERSION"
    echo "Bumped v$BEFORE -> v$VERSION"
fi

if [ "$SKIP_BUILD" -eq 0 ]; then
    # build-all.sh compiles the binary itself, then produces the .deb, the
    # portable .tar.gz and the self-extracting .run in one pass.
    bash packaging/linux/build-all.sh
    # Keep Cargo.lock in step with a bumped version.
    if [ "$VERSION" != "$BEFORE" ]; then
        cargo check --quiet >/dev/null
    fi
fi

echo
echo "Artifacts for v$VERSION:"
find dist target/debian_pkg -maxdepth 1 -type f \
    \( -name "wflow_*.deb" -o -name "wflow-*.tar.gz" -o -name "wflow-*.run" \) \
    -printf '  %f  %s bytes\n' 2>/dev/null | sort || true

# The updater verifies releases against SHA256SUMS.txt, so publish the hashes
# with the packages instead of hoping nobody edits them by hand.
if command -v sha256sum >/dev/null 2>&1; then
    (cd dist 2>/dev/null && find . -maxdepth 1 -type f \
        \( -name "wflow_*.deb" -o -name "wflow-*.tar.gz" -o -name "wflow-*.run" \) \
        -exec sha256sum {} + | sed 's|\./||' > SHA256SUMS.txt &&
        echo "Wrote dist/SHA256SUMS.txt") || true
fi

if [ "$DO_COMMIT" -eq 1 ] && [ "$VERSION" != "$BEFORE" ]; then
    git add Cargo.toml Cargo.lock
    git commit -m "release: $VERSION"
fi

if [ "$DO_TAG" -eq 1 ]; then
    git tag "v$VERSION"
    echo "Tagged v$VERSION. Push with: git push && git push --tags"
fi
