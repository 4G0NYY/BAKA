#!/usr/bin/env bash
# Renders the Scoop manifest and the AUR PKGBUILD for one release. The templates in
# packaging/ are the source of truth for everything except the version and the
# checksums, which only exist once the archives have been built and the crate published.
#
# The crate checksum is the only one a caller may not know yet: the Scoop job runs
# before anything reaches crates.io. Pass SKIP for it there and ignore the PKGBUILD
# it writes.
set -euo pipefail

if [ $# -ne 5 ]; then
    echo "usage: $0 <version> <sha256-x64> <sha256-arm64> <sha256-crate|SKIP> <output-dir>" >&2
    exit 2
fi

version=$1
x64=$2
arm64=$3
crate=$4
out=$5

root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out/aur"

render() {
    sed -e "s/{{VERSION}}/$version/g" \
        -e "s/{{SHA256_X64}}/$x64/g" \
        -e "s/{{SHA256_ARM64}}/$arm64/g" \
        -e "s/{{SHA256_CRATE}}/$crate/g" \
        "$1" > "$2"
    if grep -q '{{' "$2"; then
        echo "$2 still has a placeholder in it" >&2
        exit 1
    fi
}

render "$root/packaging/scoop/baka.json" "$out/baka.json"
render "$root/packaging/aur/PKGBUILD" "$out/aur/PKGBUILD"
