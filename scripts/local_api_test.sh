#!/bin/bash
# This script demonstrates how to start dedups in server mode locally
# and connect to it for API communication using protocol buffers

# Set log level for better visibility
export RUST_LOG=info

echo "====================================================================="
echo "  Dedups Local API Server Test"
echo "====================================================================="
echo ""

# Define the server port
SERVER_PORT=29876
CLIENT_PORT=29876

# Check if the proto feature is available
HAS_PROTO_FEATURES=$(./target/release/dedups --help | grep -- "--use-protobuf" || echo "")
if [[ -z "$HAS_PROTO_FEATURES" ]]; then
    echo "WARNING: Protocol buffer support is not available in this build."
    echo "For best results, compile with: cargo build --release --features ssh,proto"
    echo ""
fi

# Create a dummy directory for scanning
mkdir -p /tmp/api_test
touch /tmp/api_test/file1.txt
touch /tmp/api_test/file2.txt
touch /tmp/api_test/file3.txt

# Step 1: Start the server
echo "Step 1: Starting dedups server on port $SERVER_PORT"
echo "=================================================================="
echo "$ ./target/release/dedups --server-mode --port $SERVER_PORT /tmp/api_test"
echo "=================================================================="
echo "Starting server in background..."

# Start the server in background
./target/release/dedups --server-mode --port $SERVER_PORT --verbose /tmp/api_test > server.log 2>&1 &
SERVER_PID=$!
echo "Server started with PID: $SERVER_PID"
echo ""

# Give the server some time to start
sleep 1

# Check if server started correctly
if ! ps -p $SERVER_PID > /dev/null; then
    echo "ERROR: Server failed to start. Check server.log for details."
    cat server.log
    exit 1
fi

echo "Server log output:"
echo "-----------------"
cat server.log
echo "-----------------"
echo ""

# Step 2: Run a client command
echo "Step 2: Sending a command to the server"
echo "=================================================================="
echo "$ ./target/release/dedups /tmp --use-protobuf --use-compression --json --port $CLIENT_PORT"
echo "=================================================================="

# Run the client with appropriate options
if [[ -n "$HAS_PROTO_FEATURES" ]]; then
    ./target/release/dedups /tmp/api_test --port $CLIENT_PORT --use-protobuf --use-compression --json --verbose
else
    ./target/release/dedups /tmp/api_test --port $CLIENT_PORT --json --verbose
fi

CLIENT_EXIT_CODE=$?
echo ""
echo "Client command exit code: $CLIENT_EXIT_CODE"
echo ""

# Step 3: Clean up
echo "Step 3: Cleaning up"
echo "=================================================================="
echo "Stopping server (PID: $SERVER_PID)..."
kill $SERVER_PID
wait $SERVER_PID 2>/dev/null || true
echo "Server stopped"
echo ""

# Clean up test files
rm -rf /tmp/api_test
echo "Test files removed"
echo ""

echo "====================================================================="
echo "  Test Complete"
echo "====================================================================="
echo ""
echo "The above demonstration shows:"
echo "1. Starting a dedups server in API mode on a local port"
echo "2. Connecting to the server and executing a command"
echo "3. Protocol buffer communication for efficient data exchange"
echo ""
echo "This same approach is used automatically when using SSH paths"
echo "but with an SSH tunnel linking the local and remote ports."
echo ""
echo "For more advanced testing with protocol buffers, you can use:"
echo "1. Wireshark to capture and inspect protocol traffic"
echo "2. grpcurl for manual protocol buffer message testing"
echo "3. Modify this script to send custom messages"
echo ""

# Advanced demo for developers
if [[ "$1" == "--advanced" ]]; then
    echo "====================================================================="
    echo "  Advanced Protocol Testing"
    echo "====================================================================="
    echo ""
    
    # Start server again
    echo "Starting server for advanced testing..."
    ./target/release/dedups --server-mode --port $SERVER_PORT --verbose /tmp/api_test > server.log 2>&1 &
    SERVER_PID=$!
    sleep 1
    
    echo "The protocol messages follow this format:"
    echo "1. LENGTH (4 byte prefix indicating message length)"
    echo "2. MESSAGE (protocol buffer encoded message)"
    echo ""
    echo "Message structure (from proto/dedups.proto):"
    echo "- message_type: int32 (1=Command, 2=Progress, 3=Result, 4=Error)"
    echo "- payload: bytes (JSON content for the specific message type)"
    echo ""
    
    echo "Using netcat to send a raw command message to the server..."
    echo '{"command":"dedups","args":["/tmp"],"options":{"ENV_RUST_LOG":"info"}}' > /tmp/command.json
    MESSAGE='{
        "message_type": 1,
        "payload": "'$(cat /tmp/command.json | hexdump -v -e '"\\\x" 1/1 "%02x"')'"
    }'
    echo "$MESSAGE" | nc localhost $SERVER_PORT
    
    echo ""
    echo "Stopping advanced server..."
    kill $SERVER_PID
    wait $SERVER_PID 2>/dev/null || true
    rm -f /tmp/command.json
fi

exit 0 