#!/usr/bin/env bash
set -euo pipefail

bundle=${1:?Pass dmg or msi}
signing=${2:?Pass adhoc, developer-id, or unsigned}
output=${RUNNER_TEMP:-target/preview-installers}/strudeland-installer
metadata=$(cargo metadata --locked --no-deps --format-version 1)
target_dir=$(jq -jr .target_directory <<< "$metadata")
version=$(jq -jr '.packages[] | select(.name == "strudeland-app") | .version' <<< "$metadata")
test "$version" = "$(jq -jr .version src-tauri/tauri.conf.json)"

shopt -s nullglob
case "$bundle:$signing" in
  dmg:adhoc | dmg:developer-id)
    files=("$target_dir"/release/bundle/dmg/strudeland_"${version}"_aarch64.dmg)
    ;;
  msi:unsigned)
    target_dir=$(cygpath -u "$target_dir")
    output=$(cygpath -u "$output")
    files=("$target_dir"/release/bundle/msi/strudeland_"${version}"_x64_*.msi)
    ;;
  *)
    echo "Unsupported bundle and signing mode: $bundle $signing" >&2
    exit 1
    ;;
esac

if [[ ${#files[@]} -ne 1 || ! -f "${files[0]}" ]]; then
  echo "Expected exactly one Strudeland $version $bundle installer" >&2
  exit 1
fi
installer=${files[0]}

if [[ "$bundle" == dmg ]]; then
  if [[ "$signing" == developer-id ]]; then
    xcrun notarytool submit "$installer" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
      --issuer "$APPLE_API_ISSUER" --wait --timeout 30m --output-format json > "$RUNNER_TEMP/strudeland-notary.json"
    jq -e '.status == "Accepted"' "$RUNNER_TEMP/strudeland-notary.json"
    xcrun stapler staple "$installer"
    xcrun stapler validate "$installer"
  fi
  hdiutil verify "$installer"
  mountpoint=$(mktemp -d)
  trap 'hdiutil detach -quiet "$mountpoint" 2>/dev/null || true; rmdir "$mountpoint"' EXIT
  hdiutil attach -nobrowse -readonly -mountpoint "$mountpoint" "$installer"
  app="$mountpoint/strudeland.app"
  codesign --verify --deep --strict "$app"
  executable=$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$app/Contents/Info.plist")
  file "$app/Contents/MacOS/$executable" | grep -F arm64
  if [[ "$signing" == developer-id ]]; then
    xcrun stapler validate "$app"
    spctl --assess --type exec --verbose "$app"
  fi
fi

mkdir -p "$output"
cp "$installer" "$output/"
cp -R LICENSE LICENSES THIRD-PARTY-NOTICES.md "$output/"
for license in crates/*/LICENSE-* vendor/strudel/LICENSE; do
  mkdir -p "$output/$(dirname "$license")"
  cp "$license" "$output/$license"
done
source_sha=$(git rev-parse HEAD)
{
  printf 'source=https://github.com/rudoi/strudeland/tree/%s\n' "$source_sha"
  printf 'version=%s\nsigning=%s\n' "$version" "$signing"
} > "$output/BUILD.txt"
(
  cd "$output"
  if [[ "$bundle" == msi ]]; then
    sha256sum "$(basename "$installer")" > SHA256SUMS
  else
    shasum -a 256 "$(basename "$installer")" > SHA256SUMS
  fi
)
