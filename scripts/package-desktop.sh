#!/bin/sh
set -eu

if [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'This script packages the macOS desktop app.' >&2
    exit 1
fi

cd "$(dirname "$0")/.."
project_root=$(pwd)
install_root="$project_root/target/desktop-install"
app_root="$project_root/target/desktop/zerostack.app"
version=$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)

cargo install --locked --path . --debug --features desktop --root "$install_root"
mkdir -p "$app_root/Contents/MacOS"
cp "$install_root/bin/zerostack" "$app_root/Contents/MacOS/zerostack"
cat > "$app_root/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>zerostack</string>
<key>CFBundleIdentifier</key><string>org.zerostack.desktop</string>
<key>CFBundleName</key><string>zerostack</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSDocumentsFolderUsageDescription</key><string>Open the project you selected in Documents.</string>
<key>NSDesktopFolderUsageDescription</key><string>Open the project you selected on Desktop.</string>
<key>NSDownloadsFolderUsageDescription</key><string>Open the project you selected in Downloads.</string>
<key>LSEnvironment</key><dict>
<key>ZS_DESKTOP</key><string>true</string>
<key>ZS_DESKTOP_PICK_PROJECT</key><string>true</string>
</dict>
</dict></plist>
PLIST
plutil -lint "$app_root/Contents/Info.plist"
codesign --force --sign - "$app_root"
printf '%s\n' "$app_root"
