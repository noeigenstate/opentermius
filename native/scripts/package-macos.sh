#!/usr/bin/env bash
# Run on macOS with Xcode command line tools. Output is unsigned/unnotarized.
set -euo pipefail
native_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$native_root"
if [[ $(uname -s) != Darwin ]]; then
    printf '%s\n' 'This packager requires macOS.' >&2
    exit 1
fi
version=$(node scripts/check-versions.mjs)
(cd web && npm ci && npm run build)
cargo build --release --locked -p opentermius-desktop --features tauri/custom-protocol
target_root=${CARGO_TARGET_DIR:-target}
binary=${OPENTERMIUS_TEST_EXECUTABLE:-$target_root/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release/opentermius}
[[ -f $binary ]] || { printf 'Missing executable: %s\n' "$binary" >&2; exit 1; }
mkdir -p releases
stage=$(mktemp -d "${TMPDIR:-/tmp}/opentermius-macos.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
image_root=$stage/image
bundle=$image_root/OpenTermius.app
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 755 "$binary" "$bundle/Contents/MacOS/opentermius"
icon_entry=''
if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
    iconset=$stage/OpenTermius.iconset
    mkdir -p "$iconset"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" ../assets/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
        double=$((size * 2))
        sips -z "$double" "$double" ../assets/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/OpenTermius.icns"
    icon_entry='<key>CFBundleIconFile</key><string>OpenTermius.icns</string>'
else
    printf '%s\n' 'iconutil/sips unavailable: bundle uses the system default icon.' >&2
fi
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>OpenTermius</string>
<key>CFBundleDisplayName</key><string>OpenTermius</string>
<key>CFBundleIdentifier</key><string>app.opentermius.desktop</string>
<key>CFBundleExecutable</key><string>opentermius</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
$icon_entry
</dict></plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist"
ln -s /Applications "$image_root/Applications"
arch=$(uname -m)
if [[ -n ${CARGO_BUILD_TARGET:-} ]]; then arch=${CARGO_BUILD_TARGET%%-*}; fi
ditto "$bundle" "releases/OpenTermius.app"
hdiutil create -volname OpenTermius -srcfolder "$image_root" -ov -format UDZO "releases/OpenTermius-$version-macos-$arch.dmg"
printf 'Created releases/OpenTermius.app and macOS %s DMG\n' "$arch"
