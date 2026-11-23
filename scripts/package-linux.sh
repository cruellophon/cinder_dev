#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <binary_path> <output_dir> [version]" >&2
  exit 1
fi

BIN_PATH="$1"
OUT_DIR="$2"
RAW_VERSION="${3:-0.6.0}"
CLEAN_VERSION="${RAW_VERSION#v}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SVG_SRC="$REPO_ROOT/bins/bumbo-app/ui/cinder-icon.svg"

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

PKG_DIR="$STAGE_DIR/cinder-linux-x86_64"
mkdir -p "$PKG_DIR"

cp "$BIN_PATH" "$PKG_DIR/cinder"
chmod 755 "$PKG_DIR/cinder"

if [ -f "$SVG_SRC" ]; then
  cp "$SVG_SRC" "$PKG_DIR/cinder.svg"
fi

cat > "$PKG_DIR/cinder.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Cinder
GenericName=Local Agentic IDE
Comment=Local-first pure-Rust agentic IDE and GGUF model forge (${CLEAN_VERSION})
Exec=${HOME}/.local/bin/cinder
Icon=cinder
Terminal=false
Categories=Development;IDE;
EOF

cat > "$PKG_DIR/install.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
APP_DIR="$HOME/.local/share/applications"

mkdir -p "$BIN_DIR" "$ICON_DIR" "$APP_DIR"
install -m 755 "$SCRIPT_DIR/cinder" "$BIN_DIR/cinder"

if [ -f "$SCRIPT_DIR/cinder.svg" ]; then
  install -m 644 "$SCRIPT_DIR/cinder.svg" "$ICON_DIR/cinder.svg"
fi

cat > "$APP_DIR/cinder.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Cinder
GenericName=Local Agentic IDE
Comment=Local-first pure-Rust agentic IDE and GGUF model forge
Exec=$BIN_DIR/cinder
Icon=cinder
Terminal=false
Categories=Development;IDE;
DESKTOP

echo "Installed Cinder to $BIN_DIR/cinder"
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
  echo "Note: add $BIN_DIR to your PATH if 'cinder' is not found in your shell."
fi
EOF
chmod 755 "$PKG_DIR/install.sh"

ARCHIVE_PATH="$OUT_DIR/cinder-linux-x86_64.tar.gz"
rm -f "$ARCHIVE_PATH"
tar -C "$STAGE_DIR" -czf "$ARCHIVE_PATH" cinder-linux-x86_64
echo "Packaged Linux release: $ARCHIVE_PATH"
