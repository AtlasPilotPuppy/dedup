#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${GREEN}[+]${NC} $1"
}

print_error() {
    echo -e "${RED}[-]${NC} $1"
}

print_info() {
    echo -e "${BLUE}[*]${NC} $1"
}

# Build the project with all features
print_status "Building project with all features..."
cargo build --features "ssh,proto" --release

# Create test directories
print_status "Creating test directories..."
mkdir -p test_data/server
mkdir -p test_data/client

# Create test files
print_status "Creating test files..."
echo "test1" > test_data/server/test1.txt
echo "test2" > test_data/server/test2.txt
echo "test3" > test_data/server/test3.txt

# Function to test protocol
test_protocol() {
    local protocol=$1
    local compression=$2
    local level=$3
    
    print_status "Testing $protocol protocol${compression:+ with compression level $level}..."
    
    # Start server in background
    ./target/release/dedups --server-mode --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level} &
    
    SERVER_PID=$!
    
    # Wait for server to start
    sleep 2
    
    # Test 1: List files
    print_info "Test 1: Listing files..."
    ./target/release/dedups test_data/server --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level}
    
    # Test 2: Find duplicates
    print_info "Test 2: Finding duplicates..."
    ./target/release/dedups test_data/server --deduplicate --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level}
    
    # Test 3: Copy files
    print_info "Test 3: Copying files..."
    ./target/release/dedups test_data/server test_data/client --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level}
    
    # Kill server
    kill $SERVER_PID
    wait $SERVER_PID 2>/dev/null
    
    print_status "Protocol test completed"
    echo "----------------------------------------"
}

# Test JSON protocol
print_status "Testing JSON protocol..."
test_protocol "json" "" ""

# Test Protobuf without compression
print_status "Testing Protobuf without compression..."
test_protocol "protobuf" "" ""

# Test Protobuf with compression level 3
print_status "Testing Protobuf with compression level 3..."
test_protocol "protobuf" "yes" "3"

# Test Protobuf with compression level 9
print_status "Testing Protobuf with compression level 9..."
test_protocol "protobuf" "yes" "9"

# Cleanup
print_status "Cleaning up..."
rm -rf test_data

print_status "All tests completed!" 