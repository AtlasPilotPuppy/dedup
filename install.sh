#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored messages
print_message() {
    echo -e "${GREEN}[dedups]${NC} $1"
}

print_error() {
    echo -e "${RED}[dedups]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[dedups]${NC} $1"
}

# Parse command line arguments
SSH_VARIANT=false
FORCE_VARIANT=""

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --ssh) SSH_VARIANT=true ;;
        --no-ssh) SSH_VARIANT=false ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo "Options:"
            echo "  --ssh       Install the SSH-enabled variant (with remote filesystem support)"
            echo "  --no-ssh    Install the standard variant without SSH support (default)"
            echo "  --help      Show this help message"
            exit 0
            ;;
        *)
            print_error "Unknown parameter: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
    shift
done

# Function to detect OS and architecture
detect_platform() {
    local OS
    local ARCH
    local BINARY_NAME

    # Detect OS
    case "$(uname -s)" in
        Linux*)     OS="linux";;
        Darwin*)    OS="macos";;
        MINGW*|MSYS*|CYGWIN*) OS="windows";;
        *)          print_error "Unsupported operating system"; exit 1;;
    esac

    # Detect architecture
    case "$(uname -m)" in
        x86_64)     ARCH="x86_64";;
        arm64|aarch64) 
            ARCH="aarch64"
            # Only macOS supports ARM officially
            if [ "$OS" != "macos" ]; then
                print_warning "ARM architecture detected on non-macOS platform. This might not be supported."
            fi
            ;;
        *)
            print_error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac

    # Handle architecture-specific naming for macOS
    if [ "$OS" = "macos" ] && [ "$ARCH" = "aarch64" ]; then
        BINARY_NAME="dedups-macos-aarch64"
    elif [ "$OS" = "macos" ]; then
        BINARY_NAME="dedups-macos-x86_64"
    elif [ "$OS" = "linux" ]; then
        BINARY_NAME="dedups-linux-x86_64"
    elif [ "$OS" = "windows" ]; then
        BINARY_NAME="dedups-windows-x86_64.exe"
    fi

    # Add -ssh suffix if SSH variant is requested
    if [ "$SSH_VARIANT" = true ]; then
        # Check if the binary name has an extension (Windows)
        if [[ "$BINARY_NAME" == *.exe ]]; then
            BINARY_NAME="${BINARY_NAME%.exe}-ssh.exe"
        else
            BINARY_NAME="${BINARY_NAME}-ssh"
        fi
    fi

    echo "$BINARY_NAME"
}

# Get the binary name for this platform
BINARY_NAME=$(detect_platform)

# Print information about the platform
if [ "$SSH_VARIANT" = true ]; then
    print_message "Installing dedups with SSH/remote file system support..."
else
    print_message "Installing standard dedups binary..."
fi
print_message "Detected binary: $BINARY_NAME"

# Determine download URL
LATEST_RELEASE_URL="https://github.com/AtlasPilotPuppy/dedup/releases/latest/download/${BINARY_NAME}"
print_message "Downloading from: $LATEST_RELEASE_URL"

# Determine install path
DEDUPS_INSTALL_DIR=""
if [ "$(id -u)" -eq 0 ]; then
    # Root user
    DEDUPS_INSTALL_DIR="/usr/local/bin"
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    # User with sudo privileges (cached credentials)
    DEDUPS_INSTALL_DIR="/usr/local/bin"
else
    # Regular user
    if [ -d "$HOME/.local/bin" ] || mkdir -p "$HOME/.local/bin" 2>/dev/null; then
        DEDUPS_INSTALL_DIR="$HOME/.local/bin"
        if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
            print_warning "~/.local/bin is not in your PATH. You may need to add it."
            print_warning "Add this to your ~/.bashrc or ~/.zshrc:"
            print_warning "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        fi
    else
        print_error "Failed to determine installation directory"
        exit 1
    fi
fi

# Set the destination path
DEST_PATH="$DEDUPS_INSTALL_DIR/dedups"

# Download and install
print_message "Installing to: $DEST_PATH"

if [ "$(id -u)" -eq 0 ] || [ "$DEDUPS_INSTALL_DIR" = "$HOME/.local/bin" ]; then
    # Direct install
    curl -L --progress-bar "$LATEST_RELEASE_URL" -o "$DEST_PATH" || {
        print_error "Download failed"
        exit 1
    }
    chmod +x "$DEST_PATH"
else
    # Use sudo
    curl -L --progress-bar "$LATEST_RELEASE_URL" -o "/tmp/dedups_${BINARY_NAME}" || {
        print_error "Download failed"
        exit 1
    }
    chmod +x "/tmp/dedups_${BINARY_NAME}"
    sudo mv "/tmp/dedups_${BINARY_NAME}" "$DEST_PATH" || {
        print_error "Failed to move binary (sudo required)"
        exit 1
    }
fi

print_message "Installation complete! 🎉"
print_message "Run 'dedups --help' to get started."

# Show SSH info if that variant was installed
if [ "$SSH_VARIANT" = true ]; then
    print_message "SSH support is enabled. You can use commands like:"
    print_message "  dedups ssh:hostname:/path"
    print_message "  dedups /local/path ssh:hostname:/remote/path"
    print_message "See the documentation for more SSH usage examples."
fi

exit 0 