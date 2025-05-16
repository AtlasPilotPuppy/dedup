#!/bin/bash

# Test script for dedups remote operations
# This script simulates various dedups operations with remote paths

echo "=========== DEDUPS REMOTE TESTS ==========="

echo -e "\n\n1. Listing duplicates between local and remote directories (dry run):"
cargo run -- example_local example_remote --deduplicate --dry-run

echo -e "\n\n2. Listing duplicates with media deduplication (dry run):"
cargo run -- example_local/photos example_remote/photos --media-mode --media-similarity 80 --dry-run

echo -e "\n\n3. Moving duplicates from remote to local 'dupes' folder (dry run):"
mkdir -p dupes
cargo run -- example_remote/photos example_local/photos --deduplicate --move-to dupes --dry-run

echo -e "\n\n4. Deleting duplicates in remote folder keeping newest files (dry run):"
cargo run -- example_local/documents example_remote/documents --deduplicate --delete --mode newest_modified --dry-run

echo -e "\n\n5. Outputting duplicate info to JSON:"
cargo run -- example_local example_remote --deduplicate --output dedups_results.json

echo -e "\n\nTests completed. A dedups_results.json file has been created with duplicate information." 