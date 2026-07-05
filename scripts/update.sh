#!/usr/bin/env bash
# Download the latest trade-analyzer release binary for this OS into
# scripts/bin/trade-analyzer. Used by community members (no Rust needed)
# and by the Claude Code analyze workflow to keep the engine current.
#
# Usage:
#   scripts/update.sh            # latest release
#   scripts/update.sh v0.2.0     # specific tag
#   REPO=owner/name scripts/update.sh
set -euo pipefail

REPO="${REPO:-rickintveld/trading-analyst-agent}"
TAG="${1:-latest}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/bin"
DEST="$BIN_DIR/trade-analyzer"

case "$(uname -s)" in
  Darwin) ASSET="trading-analyst-agent-mac" ;;
  Linux)  ASSET="trading-analyst-agent-linux" ;;
  MINGW*|MSYS*|CYGWIN*) ASSET="trading-analyst-agent-windows.exe"; DEST="$DEST.exe" ;;
  *) echo "error: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac

if [ "$TAG" = "latest" ]; then
  URL="https://github.com/$REPO/releases/latest/download/$ASSET"
else
  URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"
fi

# Skip the download when the installed binary already matches the release tag.
if [ -x "$DEST" ] && [ "$TAG" = "latest" ] && command -v curl >/dev/null; then
  RESOLVED=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's#.*/tag/##') || RESOLVED=""
  INSTALLED=$("$DEST" --version 2>/dev/null | awk '{print $2}') || INSTALLED=""
  if [ -n "$RESOLVED" ] && [ -n "$INSTALLED" ] && [ "v$INSTALLED" = "$RESOLVED" ]; then
    echo "up to date: $DEST ($RESOLVED)"
    exit 0
  fi
fi

echo "downloading $ASSET ($TAG) from $REPO ..."
mkdir -p "$BIN_DIR"
TMP="$DEST.download"
curl -fSL --progress-bar -o "$TMP" "$URL" || {
  echo "error: download failed — no release published yet, or no asset $ASSET for tag $TAG" >&2
  rm -f "$TMP"
  exit 1
}
mv "$TMP" "$DEST"
chmod +x "$DEST"
echo "installed: $DEST"
"$DEST" --version
