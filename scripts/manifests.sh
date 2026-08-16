#!/usr/bin/env bash
# Renders the winget and Scoop manifests for one release. The templates in packaging/
# are the source of truth for everything except the version and the two checksums,
# which only exist once the archives have been built.
set -euo pipefail

if [ $# -ne 4 ]; then
    echo "usage: $0 <version> <sha256-x64> <sha256-arm64> <output-dir>" >&2
    exit 2
fi

version=$1
x64=$2
arm64=$3
out=$4

root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out/winget"

render() {
    sed -e "s/{{VERSION}}/$version/g" \
        -e "s/{{SHA256_X64}}/$x64/g" \
        -e "s/{{SHA256_ARM64}}/$arm64/g" \
        "$1" > "$2"
    if grep -q '{{' "$2"; then
        echo "$2 still has a placeholder in it" >&2
        exit 1
    fi
}

for template in "$root"/packaging/winget/*.yaml; do
    render "$template" "$out/winget/$(basename "$template")"
done
render "$root/packaging/scoop/baka.json" "$out/baka.json"
