#!/usr/bin/env bash
# Run on Linux; Tauri bundles GTK and WebKitGTK into an unsigned AppImage.
set -euo pipefail
native_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$native_root"
if [[ $(uname -s) != Linux ]]; then
    printf '%s\n' 'This packager requires Linux.' >&2
    exit 1
fi
version=$(node scripts/check-versions.mjs)
(cd web && npm ci)
target_root=${CARGO_TARGET_DIR:-target}
if [[ $target_root != /* ]]; then target_root=$native_root/$target_root; fi
export CARGO_TARGET_DIR=$target_root
export APPIMAGE_EXTRACT_AND_RUN=1
arch=$(uname -m)
case $arch in
    x86_64) bundle_arch=amd64 ;;
    aarch64) bundle_arch=aarch64 ;;
    *) printf 'Unsupported Linux architecture: %s\n' "$arch" >&2; exit 1 ;;
esac
(cd desktop && ../web/node_modules/.bin/tauri build --bundles appimage --config '{"bundle":{"active":true,"category":"Utility"}}' -- --locked)
shopt -s nullglob
appimages=("$target_root/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release/bundle/appimage/OpenTermius_${version}_"*.AppImage)
if [[ ${#appimages[@]} -ne 1 || ! -f ${appimages[0]} || ${appimages[0]} != *"_${bundle_arch}.AppImage" ]]; then
    printf 'Expected exactly one OpenTermius %s %s AppImage in the Cargo target directory.\n' "$version" "$bundle_arch" >&2
    exit 1
fi
name=OpenTermius-$version-linux-$arch.AppImage
mkdir -p releases
cp -- "${appimages[0]}" "releases/$name"
chmod 755 "releases/$name"
printf 'Created releases/%s\n' "$name"
