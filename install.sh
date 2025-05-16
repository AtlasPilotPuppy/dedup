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
        aarch64)    ARCH="aarch64";;
        arm64)      ARCH="aarch64";;
        *)          print_error "Unsupported architecture"; exit 1;;
    esac

    # Set binary name based on OS
    if [ "$OS" = "windows" ]; then
        BINARY_NAME="dedups-windows-x86_64.exe"
    else
        BINARY_NAME="dedups-${OS}-${ARCH}"
    fi

    echo "$BINARY_NAME"
}

# Function to get latest release version
get_latest_release() {
    curl -s "https://api.github.com/repos/AtlasPilotPuppy/dedup/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/'
}

# Function to install binary
install_binary() {
    local BINARY_NAME=$1
    local VERSION=$2
    local INSTALL_DIR

    # Try to install to /usr/local/bin first
    if [ -w "/usr/local/bin" ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        # Fallback to ~/.local/bin
        INSTALL_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
    fi

    print_message "Downloading dedups $VERSION..."
    curl -L "https://github.com/AtlasPilotPuppy/dedup/releases/download/${VERSION}/${BINARY_NAME}" -o "$INSTALL_DIR/dedups"

    # Make it executable
    chmod +x "$INSTALL_DIR/dedups"

    print_message "Installed dedups to $INSTALL_DIR/dedups"
    
    # Check if the binary is in PATH
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        print_warning "The installation directory is not in your PATH. Add this line to your shell configuration file:"
        echo "export PATH=\"$INSTALL_DIR:\$PATH\""
    fi
}

# Main installation process
main() {
    print_message "Starting installation..."

    # Detect platform and get binary name
    BINARY_NAME=$(detect_platform)
    if [ $? -ne 0 ]; then
        exit 1
    fi

    # Get latest release version
    VERSION=$(get_latest_release)
    if [ -z "$VERSION" ]; then
        print_error "Failed to get latest release version"
        exit 1
    fi

    # Install the binary
    install_binary "$BINARY_NAME" "$VERSION"

    print_message "Installation complete! You can now use 'dedups' command."
}

# Run main function
main 