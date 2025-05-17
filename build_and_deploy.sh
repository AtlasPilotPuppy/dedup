#!/bin/bash
set -e  # Exit on any error

echo "Starting build process..."

# Build locally first
echo "Building locally..."
cargo build --release

# Sync code to remote host (excluding target directory)
echo "Syncing code to remote host..."
rclone copy . atlas:dedup --progress --exclude="target/**" --exclude=".git/**"

# Build on remote host
echo "Building on remote host..."
ssh local "cd dedup && cargo build --release"

# Copy the binary to the final location
echo "Installing binary on remote host..."
ssh local "cp dedup/target/release/dedups ~/.local/bin/ && chmod +x ~/.local/bin/dedups"

# Verify installation
echo "Verifying remote installation..."
if ssh local "dedups --version"; then
    echo "Build and deployment completed successfully!"
else
    echo "Error: Failed to verify remote installation"
    exit 1
fi 