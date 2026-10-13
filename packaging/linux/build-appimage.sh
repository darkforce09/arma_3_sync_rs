#!/usr/bin/env bash
# Builds target/appimage/Arma3Sync-<version>-x86_64.AppImage from a release build.
# The binary links only glibc (the rest is loaded from the host at run time), so the AppImage
# holds the binary, the desktop file and the icon; it runs on systems with the build machine's
# glibc or newer.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/appimage"
app_dir="$out/AppDir"

# Pinned tools, checked against their SHA-256.
tool_url="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
tool_sha256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
runtime_url="https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
runtime_sha256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

fetch() {
    local url="$1" sha256="$2" path="$3"
    if ! echo "$sha256  $path" | sha256sum --check --status 2>/dev/null; then
        curl --fail --silent --show-error --location --output "$path" "$url"
        echo "$sha256  $path" | sha256sum --check --quiet
    fi
}

cd "$root"
cargo build --release --locked -p arma3sync
version="$(cargo metadata --format-version 1 --no-deps \
    | jq -r '.packages[] | select(.name == "arma3sync") | .version')"

rm -rf "$app_dir"
mkdir -p "$app_dir/usr/bin" "$app_dir/usr/share/applications" \
    "$app_dir/usr/share/icons/hicolor/512x512/apps"
cp target/release/arma3sync "$app_dir/usr/bin/"
cp packaging/linux/arma3sync.desktop "$app_dir/"
cp packaging/linux/arma3sync.desktop "$app_dir/usr/share/applications/"
cp crates/viewer/a3s_ui/assets/app-icon.png "$app_dir/arma3sync.png"
cp crates/viewer/a3s_ui/assets/app-icon.png "$app_dir/usr/share/icons/hicolor/512x512/apps/arma3sync.png"
ln -s usr/bin/arma3sync "$app_dir/AppRun"

fetch "$tool_url" "$tool_sha256" "$out/appimagetool"
fetch "$runtime_url" "$runtime_sha256" "$out/runtime"
chmod +x "$out/appimagetool"

image="$out/Arma3Sync-$version-x86_64.AppImage"
# CI runners have no FUSE; the tool unpacks itself instead.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$out/appimagetool" --no-appstream \
    --runtime-file "$out/runtime" "$app_dir" "$image"
echo "$image"
