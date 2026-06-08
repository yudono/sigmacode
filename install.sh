#!/usr/bin/env bash
set -euo pipefail

# SigmaCode Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/yudono/sigmacode/main/install.sh | bash

BINARY_NAME="sigmacode"
REPO="yudono/sigmacode"
INSTALL_DIR="${SIGMACODE_INSTALL_DIR:-$HOME/.local/bin}"
GITHUB_API="https://api.github.com/repos/$REPO/releases/latest"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[sigmacode]${NC} $1"; }
warn()  { echo -e "${YELLOW}[sigmacode]${NC} $1"; }
error() { echo -e "${RED}[sigmacode]${NC} $1" >&2; exit 1; }

detect_os() {
    local os
    os="$(uname -s)"
    case "$os" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "darwin" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *) error "Unsupported OS: $os" ;;
    esac
}

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64)   echo "x86_64" ;;
        aarch64|arm64)   echo "aarch64" ;;
        armv7l|armhf)    echo "armv7" ;;
        *) error "Unsupported architecture: $arch" ;;
    esac
}

get_download_url() {
    local os="$1" arch="$2"
    local platform="${os}"

    case "$os" in
        darwin)  platform="apple-darwin" ;;
        linux)   platform="unknown-linux-gnu" ;;
        windows) platform="pc-windows-msvc" ;;
    esac

    local archive_name="${BINARY_NAME}-${arch}-${platform}.tar.gz"
    echo "https://github.com/${REPO}/releases/latest/download/${archive_name}"
}

main() {
    info "Installing SigmaCode..."

    local os arch
    os="$(detect_os)"
    arch="$(detect_arch)"
    info "Detected: $os $arch"

    local url
    url="$(get_download_url "$os" "$arch")"
    info "Downloading from: $url"

    mkdir -p "$INSTALL_DIR"

    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap "rm -rf $tmp_dir" EXIT

    info "Downloading..."
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$tmp_dir/sigmacode.tar.gz"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$tmp_dir/sigmacode.tar.gz"
    else
        error "Neither curl nor wget found. Please install one."
    fi

    info "Extracting..."
    tar -xzf "$tmp_dir/sigmacode.tar.gz" -C "$tmp_dir"

    info "Installing to $INSTALL_DIR/$BINARY_NAME"
    mv "$tmp_dir/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    # Check if install dir is in PATH
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        warn "$INSTALL_DIR is not in your PATH."
        warn "Add this to your shell profile:"
        warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi

    info "SigmaCode installed successfully!"
    info "Run: sigmacode --help"

    # Create config directory
    mkdir -p "$HOME/.sigma"
    if [ ! -f "$HOME/.sigma/config.yml" ]; then
        info "Creating default config at ~/.sigma/config.yml"
        cat > "$HOME/.sigma/config.yml" << 'EOF'
provider: openai
model: gpt-4o
base_url: https://api.openai.com/v1
workspace: .
EOF
    fi
}

main "$@"
