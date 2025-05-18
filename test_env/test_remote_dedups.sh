#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}Setting up test environment...${NC}"

# Create test directory structure in container
ssh test-dedups "mkdir -p /home/testuser/test_data/pictures"
ssh test-dedups "dd if=/dev/urandom of=/home/testuser/test_data/pictures/file1.jpg bs=1M count=1"
ssh test-dedups "cp /home/testuser/test_data/pictures/file1.jpg /home/testuser/test_data/pictures/file2.jpg"
ssh test-dedups "dd if=/dev/urandom of=/home/testuser/test_data/pictures/file3.jpg bs=1M count=2"

# Add more files for better testing of progress
ssh test-dedups "mkdir -p /home/testuser/test_data/documents"
for i in {1..5}; do
    ssh test-dedups "dd if=/dev/urandom of=/home/testuser/test_data/documents/doc$i.txt bs=1K count=$i"
done

# Create some duplicate files
ssh test-dedups "cp /home/testuser/test_data/documents/doc1.txt /home/testuser/test_data/documents/doc1_copy.txt"
ssh test-dedups "cp /home/testuser/test_data/documents/doc2.txt /home/testuser/test_data/documents/doc2_copy.txt"

echo -e "${YELLOW}Testing basic remote dedups functionality...${NC}"

# Test 1: Basic remote scan with standard SSH (no tunnel)
echo -e "\n${GREEN}Test 1: Basic remote scan with JSON output (no tunnel)${NC}"
RUST_LOG=debug ./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" --json --no-use-ssh-tunnel

# Verify the JSON output
echo -e "\n${YELLOW}Verifying standard SSH JSON output...${NC}"
if ! OUTPUT=$(./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" --json --no-use-ssh-tunnel); then
    echo -e "${RED}Standard SSH JSON output test failed!${NC}"
    exit 1
fi

# Test 2: Basic remote scan with SSH tunnel
echo -e "\n${GREEN}Test 2: Basic remote scan with JSON output (using SSH tunnel)${NC}"
RUST_LOG=debug ./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" --json --use-ssh-tunnel

# Verify the tunnel JSON output
echo -e "\n${YELLOW}Verifying tunneled SSH JSON output...${NC}"
if ! OUTPUT=$(./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" --json --use-ssh-tunnel); then
    echo -e "${RED}Tunnel SSH JSON output test failed!${NC}"
    exit 1
fi

# Check for key elements of streaming JSON output:
# 1. Progress updates
if ! echo "$OUTPUT" | grep -q '"type":"progress"'; then
    echo -e "${RED}Missing progress updates in JSON output${NC}"
    exit 1
fi

# 2. Result information
if ! echo "$OUTPUT" | grep -q '"type":"result"'; then
    echo -e "${RED}Missing result information in JSON output${NC}"
    exit 1
fi

# 3. Correct count of files
if ! echo "$OUTPUT" | grep -q '"total_files"'; then
    echo -e "${RED}Missing total_files in JSON output${NC}"
    exit 1
fi

echo -e "${GREEN}JSON output validation successful${NC}"

# Test 3: Media mode with progress
echo -e "\n${GREEN}Test 3: Media mode with progress${NC}"
RUST_LOG=debug ./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" \
    --media-mode \
    --media-similarity 85 \
    --media-resolution highest \
    --media-formats jpg \
    --progress

# Test 4: JSON streaming with multi-directory processing
echo -e "\n${GREEN}Test 4: JSON streaming with multi-directory processing${NC}"
# Create a second directory for comparison
ssh test-dedups "mkdir -p /home/testuser/test_data2/documents"
# Copy some files to create duplicates across directories
ssh test-dedups "cp /home/testuser/test_data/documents/doc1.txt /home/testuser/test_data2/documents/"
ssh test-dedups "cp /home/testuser/test_data/pictures/file1.jpg /home/testuser/test_data2/"
# Add a unique file
ssh test-dedups "dd if=/dev/urandom of=/home/testuser/test_data2/unique.bin bs=1K count=10"

# Run multi-directory comparison with JSON output
echo -e "\n${YELLOW}Testing multi-directory JSON streaming...${NC}"
OUTPUT=$(./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" "ssh:test-dedups:/home/testuser/test_data2" --json)

# Test 5: Interactive features with JSON streaming
echo -e "\n${GREEN}Test 5: JSON streaming with duplicate action${NC}"
RUST_LOG=debug ./target/debug/dedups "ssh:test-dedups:/home/testuser/test_data" \
    --json \
    --deduplicate \
    --move-to "/home/testuser/moved_files" \
    --dry-run

# Clean up
echo -e "\n${YELLOW}Cleaning up test data...${NC}"
ssh test-dedups "rm -rf /home/testuser/test_data/pictures"
ssh test-dedups "rm -rf /home/testuser/test_data/documents"
ssh test-dedups "rm -rf /home/testuser/test_data2"
ssh test-dedups "rm -rf /home/testuser/moved_files"

echo -e "${GREEN}All tests completed!${NC}" 