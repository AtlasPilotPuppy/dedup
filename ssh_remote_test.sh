#!/bin/bash

# Script to test SSH remote operations with dedups
# This script will set up a simulated remote environment and run various dedups commands

echo "Setting up test environment..."

# Create test directories
mkdir -p ssh_test_local/photos ssh_test_remote/photos
mkdir -p ssh_test_local/documents ssh_test_remote/documents

# Copy test files
cp test_local/media/* ssh_test_local/photos/
cp test_remote/media/real_images/* ssh_test_remote/photos/
cp test_local/file1_same.txt ssh_test_local/documents/
cp test_remote/documents/file1.txt ssh_test_remote/documents/

# Create a few unique files
echo "Unique file in local" > ssh_test_local/documents/unique_local.txt
echo "Unique file in remote" > ssh_test_remote/documents/unique_remote.txt
cp ssh_test_local/photos/red.png ssh_test_remote/photos/red_duplicate.png

echo -e "\n=== Test Environment Created ===\n"
echo "Local test directory: $(pwd)/ssh_test_local"
echo "Remote test directory: $(pwd)/ssh_test_remote"

# Show example commands
echo -e "\n=== Example Commands to Test SSH Integration ===\n"
echo "1. List duplicates between local and remote (dry run):"
echo "   cargo run -- ssh_test_local ssh_test_remote --deduplicate --dry-run"
echo ""
echo "2. Find and move duplicates to archive (dry run):"
echo "   mkdir -p ssh_test_archive"
echo "   cargo run -- ssh_test_local ssh_test_remote --deduplicate --move-to ssh_test_archive --dry-run"
echo ""
echo "3. Media deduplication on photos directories (dry run):"
echo "   cargo run -- ssh_test_local/photos ssh_test_remote/photos --media-mode --media-similarity 80 --dry-run"
echo ""
echo "4. Output duplicates to JSON:"
echo "   cargo run -- ssh_test_local ssh_test_remote --deduplicate --output ssh_test_duplicates.json"
echo ""
echo "5. Delete duplicates from remote keeping newest files (dry run):"
echo "   cargo run -- ssh_test_local/documents ssh_test_remote/documents --deduplicate --delete --mode newest_modified --dry-run"
echo ""
echo "Note: To actually perform operations, remove the --dry-run flag" 