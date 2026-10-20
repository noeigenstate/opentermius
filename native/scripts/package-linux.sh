#!/usr/bin/env bash
# A portable archive; dynamic libraries and a desktop portal remain host requirements.
set -euo pipefail
native_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$native_root"
if [[ $(uname -s) != Linux ]]; then
    printf '%s\n' 'This packager requires Linux.' >&2
    exit 1
fi
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
cargo build --release --locked
target_root=${CARGO_TARGET_DIR:-target}
binary=${OPENTERMIUS_TEST_EXECUTABLE:-$target_root/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release/opentermius}
[[ -f $binary ]] || { printf 'Missing executable: %s\n' "$binary" >&2; exit 1; }
arch=$(uname -m)
if [[ -n ${CARGO_BUILD_TARGET:-} ]]; then arch=${CARGO_BUILD_TARGET%%-*}; fi
name=OpenTermius-$version-linux-$arch
mkdir -p releases
stage=$(mktemp -d "${TMPDIR:-/tmp}/opentermius-linux.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
root=$stage/$name
mkdir -p "$root/bin" "$root/share/applications" "$root/share/icons/hicolor/512x512/apps"
install -m 755 "$binary" "$root/bin/opentermius"
install -m 644 ../assets/icon.png "$root/share/icons/hicolor/512x512/apps/opentermius.png"
cat > "$root/share/applications/opentermius.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=OpenTermius
Comment=Local-first SSH and SFTP client
Exec=opentermius
Icon=opentermius
Terminal=false
Categories=Network;TerminalEmulator;
StartupWMClass=opentermius
DESKTOP
cat > "$root/README.txt" <<'README'
Run bin/opentermius from this extracted directory.
To install per-user, copy bin/opentermius to ~/.local/bin, and copy the
share/applications and share/icons contents into the matching ~/.local/share
directories. Ensure ~/.local/bin is on PATH for the desktop launcher.
Requires system X11/Wayland, fontconfig, xkbcommon, fonts, and an XDG desktop
portal (with a desktop-specific implementation) for file dialogs.
Saved secrets use AES-256-GCM and a 0600 master-key-v1 file in the OpenTermius
config directory. This file is not protected by an OS keyring. Keep it private
and back it up with the config; losing it makes saved secrets unrecoverable.
README
tar -C "$stage" -czf "releases/$name.tar.gz" "$name"
printf 'Created releases/%s.tar.gz\n' "$name"
