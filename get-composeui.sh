#!/usr/bin/env sh
set -eu

REPO="${COMPOSEUI_REPO:-gu1p/composer-ui-rs}"
BIN_NAME="${COMPOSEUI_BIN:-composeui}"
INSTALL_DIR="${COMPOSEUI_INSTALL_DIR:-}"
VERSION="${COMPOSEUI_VERSION:-latest}"

usage() {
  cat <<EOF
Usage: get-composeui.sh [--version <version>] [--repo <owner/repo>] [--install-dir <path>]

Environment overrides:
  COMPOSEUI_REPO=owner/repo
  COMPOSEUI_BIN=composeui
  COMPOSEUI_INSTALL_DIR=~/.local/bin
  COMPOSEUI_VERSION=latest
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

fetch() {
  url="$1"
  out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
    return
  fi
  fail "curl or wget is required"
}

latest_tag() {
  url="https://github.com/$REPO/releases/latest"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSLI "$url" \
      | tr -d '\r' \
      | awk -F/ '/^location:/ {print $NF; exit}'
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qS --spider "$url" 2>&1 \
      | tr -d '\r' \
      | awk -F/ '/[Ll]ocation:/ {print $NF; exit}'
    return
  fi
  fail "curl or wget is required"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      VERSION="$2"
      shift 2
      ;;
    --repo)
      [ $# -ge 2 ] || fail "--repo requires a value"
      REPO="$2"
      shift 2
      ;;
    --install-dir)
      [ $# -ge 2 ] || fail "--install-dir requires a value"
      INSTALL_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

if [ -z "$INSTALL_DIR" ]; then
  if [ "$(id -u)" -eq 0 ]; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="$HOME/.local/bin"
  fi
fi

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) platform="apple-darwin" ;;
  Linux) platform="unknown-linux-gnu" ;;
  *) fail "unsupported OS: $OS" ;;
esac

case "$ARCH" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) fail "unsupported architecture: $ARCH" ;;
esac

target="$arch-$platform"

if [ "$VERSION" = "latest" ]; then
  tag="$(latest_tag)"
  [ -n "$tag" ] || fail "failed to resolve latest release tag"
else
  case "$VERSION" in
    v*) tag="$VERSION" ;;
    *) tag="v$VERSION" ;;
  esac
fi

version="${tag#v}"
asset="${BIN_NAME}-${version}-${target}"
url="https://github.com/$REPO/releases/download/$tag/$asset"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
tmpfile="$tmpdir/$asset"

echo "Downloading $url"
fetch "$url" "$tmpfile"
chmod +x "$tmpfile"

mkdir -p "$INSTALL_DIR"
install_path="$INSTALL_DIR/${BIN_NAME}-${version}"
mv "$tmpfile" "$install_path"
ln -sf "$install_path" "$INSTALL_DIR/$BIN_NAME"

echo "Installed $install_path"
echo "Symlinked $INSTALL_DIR/$BIN_NAME -> $install_path"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
