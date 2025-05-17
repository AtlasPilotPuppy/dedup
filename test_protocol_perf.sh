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

# Create test files of different sizes
print_status "Creating test files..."
for i in {1..100}; do
    dd if=/dev/urandom of="test_data/server/file$i.txt" bs=1M count=1 2>/dev/null
done

# Function to measure time
time_command() {
    local start_time=$(date +%s.%N)
    # Run the command and capture its output
    local output=$("$@")
    local end_time=$(date +%s.%N)
    # Calculate elapsed time
    local elapsed=$(echo "$end_time - $start_time" | bc)
    # Return both the elapsed time and the command output
    echo "$elapsed"
}

# Function to test protocol with performance measurement
test_protocol_perf() {
    local protocol=$1
    local compression=$2
    local level=$3
    
    print_status "Testing $protocol protocol${compression:+ with compression level $level}..."
    
    # Build server command
    local server_cmd="./target/release/dedups --server-mode --port 12345"
    if [[ "$protocol" == "protobuf" ]]; then
        server_cmd+=" --use-protobuf"
        if [[ "$compression" == "yes" ]]; then
            server_cmd+=" --use-compression --compression-level $level"
        fi
    fi
    
    # Start server in background
    eval "$server_cmd &"
    SERVER_PID=$!
    
    # Wait for server to start
    sleep 2
    
    # Test 1: List files (measure time)
    print_info "Test 1: Listing files..."
    local list_time=$(time_command ./target/release/dedups test_data/server --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level})
    print_info "List time: ${list_time}s"
    
    # Test 2: Find duplicates (measure time)
    print_info "Test 2: Finding duplicates..."
    local dedup_time=$(time_command ./target/release/dedups test_data/server --deduplicate --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level})
    print_info "Deduplication time: ${dedup_time}s"
    
    # Test 3: Copy files (measure time)
    print_info "Test 3: Copying files..."
    local copy_time=$(time_command ./target/release/dedups test_data/server test_data/client --json --port 12345 \
        ${protocol:+--use-protobuf} \
        ${compression:+--use-compression} \
        ${level:+--compression-level $level})
    print_info "Copy time: ${copy_time}s"
    
    # Calculate total time
    local total_time=$(echo "$list_time + $dedup_time + $copy_time" | bc)
    print_info "Total time: ${total_time}s"
    
    # Kill server
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
    
    print_status "Protocol test completed"
    echo "----------------------------------------"
}

# Create results file
echo "Protocol Test Results" > protocol_test_results.txt
echo "====================" >> protocol_test_results.txt
echo "" >> protocol_test_results.txt

# Test JSON protocol
print_status "Testing JSON protocol..."
test_protocol_perf "json" "" "" | tee -a protocol_test_results.txt

# Test Protobuf without compression
print_status "Testing Protobuf without compression..."
test_protocol_perf "protobuf" "" "" | tee -a protocol_test_results.txt

# Test Protobuf with different compression levels
for level in 1 3 6 9 12 15 18 21; do
    print_status "Testing Protobuf with compression level $level..."
    test_protocol_perf "protobuf" "yes" "$level" | tee -a protocol_test_results.txt
done

# Cleanup
print_status "Cleaning up..."
rm -rf test_data

print_status "All tests completed! Results saved to protocol_test_results.txt" 