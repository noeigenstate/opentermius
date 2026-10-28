#!/bin/bash
# Installs or updates OpenTermius on a Mac with Apple silicon, from the latest GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/noeigenstate/opentermius/main/native/scripts/install-macos.sh | bash
# The build is signed ad hoc, not notarized by Apple. A file curl downloads carries no quarantine flag, so the
# app opens directly, without Gatekeeper's "cannot verify the developer" prompt; a DMG downloaded in a browser
# does show it. Nothing needs an administrator password.
# Options (environment variables):
#   OPENTERMIUS_VERSION=x.y.z          a given release instead of the latest
#   OPENTERMIUS_INSTALL_DIR=~/Applications   another folder than /Applications
#   OPENTERMIUS_OPEN=0                 install without opening the app
# A downloaded zip can be installed directly: bash install-macos.sh ~/Downloads/OpenTermius-x.y.z-macos-arm64.zip
set -euo pipefail

repo=noeigenstate/opentermius
app_name='OpenTermius.app'
fail() { printf 'OpenTermius: %s\n' "$1" >&2; exit 1; }

[[ $(uname -s) == Darwin ]] || fail 'this installer is for macOS.'
[[ $(sysctl -n hw.optional.arm64 2>/dev/null || true) == 1 ]] || fail 'this build needs a Mac with Apple silicon (M1 or later).'

work=$(mktemp -d "${TMPDIR:-/tmp}/opentermius-install.XXXXXX")
trap 'rm -rf "$work"' EXIT

if [[ $# -ge 1 ]]; then
  archive=$1
  [[ -f $archive ]] || fail "no such file: $archive"
else
  if [[ -n ${OPENTERMIUS_VERSION:-} ]]; then
    tag="v${OPENTERMIUS_VERSION#v}"
  else
    # The latest release page redirects to its tag; no API token or rate limit involved.
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") || fail 'could not reach GitHub.'
    tag=${latest##*/}
  fi
  [[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || fail "no release found ($tag)."
  version=${tag#v}
  name="OpenTermius-$version-macos-arm64.zip"
  base="https://github.com/$repo/releases/download/$tag"
  printf 'OpenTermius: downloading %s…\n' "$version"
  curl -fL --progress-bar -o "$work/$name" "$base/$name" || fail "$tag has no macOS build."
  curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS" || fail 'the checksum file is missing from the release.'
  expected=$(awk -v file="$name" '$2 == file { print $1 }' "$work/SHA256SUMS")
  actual=$(shasum -a 256 "$work/$name" | awk '{ print $1 }')
  [[ -n $expected && $expected == "$actual" ]] || fail 'the download does not match its SHA-256 checksum.'
  archive="$work/$name"
fi

ditto -x -k "$archive" "$work/app"
[[ -d "$work/app/$app_name" ]] || fail "the archive does not contain $app_name."
codesign --verify --deep --strict "$work/app/$app_name" 2>/dev/null || fail 'the application signature is broken.'

destination=${OPENTERMIUS_INSTALL_DIR:-/Applications}
destination=${destination/#\~/$HOME}
if [[ -z ${OPENTERMIUS_INSTALL_DIR:-} && ! -w /Applications ]]; then destination="$HOME/Applications"; fi
mkdir -p "$destination"
target="$destination/$app_name"

# A running copy is asked to quit first; nothing is replaced until it has.
if pgrep -f "/$app_name/Contents/MacOS/opentermius" >/dev/null 2>&1; then
  printf 'OpenTermius: quitting the running copy…\n'
  osascript -e 'tell application "OpenTermius" to quit' >/dev/null 2>&1 || fail 'OpenTermius is running. Quit it, then run this again.'
  for _ in $(seq 1 120); do pgrep -f "/$app_name/Contents/MacOS/opentermius" >/dev/null 2>&1 || break; sleep 1; done
  pgrep -f "/$app_name/Contents/MacOS/opentermius" >/dev/null 2>&1 && fail 'OpenTermius is still running. Quit it, then run this again.'
fi

rm -rf "$target"
ditto "$work/app/$app_name" "$target"
# A zip that came through a browser passes its quarantine flag on; this copy is trusted as installed.
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
printf 'OpenTermius: installed %s\n' "$target"
[[ ${OPENTERMIUS_OPEN:-1} == 0 ]] || open "$target"
