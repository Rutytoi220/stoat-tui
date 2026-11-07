#!/bin/sh
# Single-command installer for stoat-tui
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/stoat-chat/stoat-tui/main/scripts/install.sh | sh
#
# Environment overrides:
#   REPO="stoat-chat/stoat-tui"
#   VERSION="v0.1.0"
#   INSTALL_DIR="/usr/local/bin" (or ~/.local/bin)

set -eu

REPO="${REPO:-stoat-chat/stoat-tui}"
GITHUB_URL="https://github.com"
GITHUB_API="https://api.github.com"

info() {
    printf "\033[1;34m::\033[0m %s\n" "$1"
}

success() {
    printf "\033[1;32m✓\033[0m %s\n" "$1"
}

warn() {
    printf "\033[1;33m!\033[0m %s\n" "$1"
}

error() {
    printf "\033[1;31m✗ Error:\033[0m %s\n" "$1" >&2
    exit 1
}

# 1. Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)
        OS_TYPE="linux"
        ;;
    *)
        error "Unsupported operating system: $OS. stoat-tui currently provides prebuilt binaries for Linux."
        ;;
esac

# 2. Detect Architecture & Target
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)
        TARGET="x86_64-unknown-linux-musl"
        FALLBACK_TARGET="x86_64-unknown-linux-gnu"
        ;;
    aarch64|arm64)
        TARGET="aarch64-unknown-linux-gnu"
        FALLBACK_TARGET=""
        ;;
    *)
        error "Unsupported architecture: $ARCH. Supported architectures: x86_64, aarch64."
        ;;
esac

info "Detected platform: $OS ($ARCH) -> target: $TARGET"

# 3. Determine Version
if [ -z "${VERSION:-}" ]; then
    info "Fetching latest release tag from GitHub..."
    LATEST_JSON=$(curl -fsSL "${GITHUB_API}/repos/${REPO}/releases/latest" 2>/dev/null || true)
    if [ -n "$LATEST_JSON" ]; then
        VERSION=$(printf '%s' "$LATEST_JSON" | grep '"tag_name":' | head -n1 | sed -E 's/.*"([^"]+)".*/\1/')
    fi

    if [ -z "${VERSION:-}" ]; then
        VERSION="v0.1.0"
        warn "Could not fetch latest release tag from GitHub API, falling back to $VERSION"
    fi
fi

info "Installing stoat-tui $VERSION..."

# 4. Determine Download URL & Filename
ARCHIVE="stoat-tui-${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="${GITHUB_URL}/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
CHECKSUM_URL="${GITHUB_URL}/${REPO}/releases/download/${VERSION}/checksums.txt"

TMP_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t 'stoat-tui')
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

info "Downloading ${ARCHIVE}..."
if ! curl -fsSL "$DOWNLOAD_URL" -o "${TMP_DIR}/${ARCHIVE}" 2>/dev/null; then
    if [ -n "$FALLBACK_TARGET" ]; then
        warn "Primary target ${TARGET} not found, trying fallback ${FALLBACK_TARGET}..."
        TARGET="$FALLBACK_TARGET"
        ARCHIVE="stoat-tui-${VERSION}-${TARGET}.tar.gz"
        DOWNLOAD_URL="${GITHUB_URL}/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
        if ! curl -fsSL "$DOWNLOAD_URL" -o "${TMP_DIR}/${ARCHIVE}"; then
            error "Failed to download release archive from $DOWNLOAD_URL"
        fi
    else
        error "Failed to download release archive from $DOWNLOAD_URL"
    fi
fi

# 5. Verify Checksum if available
info "Verifying archive integrity..."
if curl -fsSL "$CHECKSUM_URL" -o "${TMP_DIR}/checksums.txt" 2>/dev/null; then
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$TMP_DIR" && grep "$ARCHIVE" checksums.txt | sha256sum -c - >/dev/null 2>&1) && success "SHA-256 Checksum verified." || warn "Checksum check failed or archive not in checksums.txt, proceeding with caution."
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$TMP_DIR" && grep "$ARCHIVE" checksums.txt | shasum -a 256 -c - >/dev/null 2>&1) && success "SHA-256 Checksum verified." || warn "Checksum check failed, proceeding with caution."
    fi
else
    warn "Checksums file not found on release, skipping checksum verification."
fi

# 6. Extract Binary
info "Extracting archive..."
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"

BINARY_PATH=$(find "$TMP_DIR" -type f -name "stoat-tui" | head -n 1)
if [ -z "$BINARY_PATH" ] || [ ! -f "$BINARY_PATH" ]; then
    error "Binary 'stoat-tui' not found in extracted archive."
fi

# 7. Determine Destination Directory
if [ -n "${INSTALL_DIR:-}" ]; then
    DEST_DIR="$INSTALL_DIR"
elif [ "$(id -u)" -eq 0 ]; then
    DEST_DIR="/usr/local/bin"
elif [ -w "/usr/local/bin" ]; then
    DEST_DIR="/usr/local/bin"
else
    DEST_DIR="${HOME}/.local/bin"
fi

mkdir -p "$DEST_DIR"
chmod 755 "$DEST_DIR" 2>/dev/null || true

info "Installing binary to ${DEST_DIR}/stoat-tui..."
cp "$BINARY_PATH" "${DEST_DIR}/stoat-tui"
chmod +x "${DEST_DIR}/stoat-tui"

# 8. Verify Installation
if [ -x "${DEST_DIR}/stoat-tui" ]; then
    success "Successfully installed stoat-tui to ${DEST_DIR}/stoat-tui"
    "${DEST_DIR}/stoat-tui" --version || true
else
    error "Installation verification failed. ${DEST_DIR}/stoat-tui is not executable."
fi

# Check if DEST_DIR is in PATH
case ":$PATH:" in
    *":${DEST_DIR}:"*)
        ;;
    *)
        warn "${DEST_DIR} is not in your \$PATH."
        warn "Add it to your shell profile (e.g. ~/.bashrc or ~/.zshrc):"
        printf "    export PATH=\"%s:\$PATH\"\n\n" "$DEST_DIR"
        ;;
esac

info "Run 'stoat-tui' to start chatting!"
