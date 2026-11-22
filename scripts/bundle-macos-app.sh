#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 3 ]; then
  echo "Usage: $0 <binary_path> <output_dir> <arch_label> [version]" >&2
  exit 1
fi

BIN_PATH="$1"
OUT_DIR="$2"
ARCH_LABEL="$3"
RAW_VERSION="${4:-0.5.5}"
CLEAN_VERSION="${RAW_VERSION#v}"
BUNDLE_NUMERIC_VERSION="0.5.5"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ICO_SRC="$REPO_ROOT/bins/bumbo-app/ui/cinder.ico"

if [ ! -f "$BIN_PATH" ]; then
  echo "Binary not found: $BIN_PATH" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
STAGE_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$STAGE_DIR"
}
trap cleanup EXIT

APP_DIR="$STAGE_DIR/Cinder.app"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/cinder"
chmod 755 "$APP_DIR/Contents/MacOS/cinder"

cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Cinder</string>
  <key>CFBundleDisplayName</key>
  <string>Cinder</string>
  <key>CFBundleIdentifier</key>
  <string>dev.cinder.ide</string>
  <key>CFBundleExecutable</key>
  <string>cinder</string>
  <key>CFBundleIconFile</key>
  <string>Cinder</string>
  <key>CFBundleVersion</key>
  <string>${BUNDLE_NUMERIC_VERSION}</string>
  <key>CFBundleShortVersionString</key>
  <string>${CLEAN_VERSION}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key>
  <true/>
</dict>
</plist>
EOF

# Generate macOS .icns icon from bins/bumbo-app/ui/cinder.ico using native macOS tools.
if [ -f "$ICO_SRC" ] && command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
  (
    ICON_PNG="$STAGE_DIR/cinder_base.png"
    ICONSET_DIR="$STAGE_DIR/Cinder.iconset"
    mkdir -p "$ICONSET_DIR"
    if sips -s format png "$ICO_SRC" --out "$ICON_PNG" >/dev/null 2>&1; then
      for sz in 16 32 128 256 512; do
        sips -z "$sz" "$sz" "$ICON_PNG" --out "$ICONSET_DIR/icon_${sz}x${sz}.png" >/dev/null 2>&1 || true
        sz2=$((sz * 2))
        sips -z "$sz2" "$sz2" "$ICON_PNG" --out "$ICONSET_DIR/icon_${sz}x${sz}@2x.png" >/dev/null 2>&1 || true
      done
      iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/Cinder.icns" >/dev/null 2>&1 || true
    fi
  ) || true
fi

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_DIR"
fi

ARCHIVE_PATH="$OUT_DIR/cinder-macos-${ARCH_LABEL}.zip"
rm -f "$ARCHIVE_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ARCHIVE_PATH"
echo "Packaged macOS app bundle: $ARCHIVE_PATH"
