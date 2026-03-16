#!/bin/sh
# pylot installer
# Usage: curl -fsSL https://raw.githubusercontent.com/gedaliahs/pylot/main/install.sh | sh
set -e

REPO="gedaliahs/pylot"
INSTALL_DIR="${PYLOT_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="pylot"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

info() { printf "${CYAN}>${RESET} %s\n" "$1"; }
success() { printf "${GREEN}>${RESET} %s\n" "$1"; }
error() { printf "${RED}error${RESET}: %s\n" "$1" >&2; exit 1; }

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        *)       error "Unsupported OS: $(uname -s). pylot supports Linux and macOS." ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *)             error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Get latest release tag from GitHub
get_latest_version() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    else
        error "curl or wget is required to install pylot"
    fi
}

# Download a file
download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    fi
}

main() {
    printf "\n${BOLD}${CYAN}  pylot installer${RESET}\n\n"

    OS=$(detect_os)
    ARCH=$(detect_arch)
    VERSION=$(get_latest_version)

    if [ -z "$VERSION" ]; then
        error "Could not determine latest version. Check https://github.com/${REPO}/releases"
    fi

    info "Detected: ${OS} ${ARCH}"
    info "Version:  ${VERSION}"

    # Construct download URL
    ARCHIVE_NAME="pylot-${VERSION}-${ARCH}-${OS}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    info "Downloading ${DOWNLOAD_URL}..."
    download "$DOWNLOAD_URL" "$TMP_DIR/$ARCHIVE_NAME" || error "Download failed. Check that the release exists at:\n  $DOWNLOAD_URL"

    # Extract
    info "Extracting..."
    tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$TMP_DIR"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    success "Installed pylot ${VERSION} to ${INSTALL_DIR}/${BINARY_NAME}"

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            printf "\n"
            info "Add pylot to your PATH by adding this to your shell profile:"
            printf "\n  ${BOLD}export PATH=\"%s:\$PATH\"${RESET}\n" "$INSTALL_DIR"
            ;;
    esac

    # Shell integration hint
    printf "\n"
    info "To enable shell integration (cd, env switching), add to your shell profile:"
    printf "\n  ${BOLD}eval \"\$(pylot shell-init zsh)\"${RESET}  # or bash/fish\n"
    printf "\n"
    success "Done! Run 'pylot --help' to get started."
    printf "\n"
}

main
