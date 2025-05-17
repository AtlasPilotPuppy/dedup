#!/bin/bash
# This script demonstrates how to use the SSH tunnel API communication mode correctly
# with dedups to ensure it communicates over the tunnel rather than parsing stdout.

# Set verbosity for detailed logs
export RUST_LOG=info

# Print banner
echo "====================================================================="
echo "  Dedups SSH API Communication Example"
echo "====================================================================="
echo ""

# Replace these with your actual SSH remote path and SSH username 
SSH_HOST="local"  # Replace with your SSH host
REMOTE_PATH="/mnt/raid/imports/Pictures/presidentsDay"  # Replace with your remote path

# Construct the full SSH path
SSH_PATH="ssh:$SSH_HOST:$REMOTE_PATH"

echo "Using remote path: $SSH_PATH"
echo ""

# Check which features are available
HAS_SSH_FEATURES=$(./target/release/dedups --help | grep -- "--tunnel-api-mode" || echo "")
HAS_PROTO_FEATURES=$(./target/release/dedups --help | grep -- "--use-protobuf" || echo "")

# First command: Run with tunnel API mode and protobuf explicitly enabled if available
echo "Running with proper API communication mode:"
echo "=================================================================="

# Build command based on available features
CMD="./target/release/dedups \"$SSH_PATH\" -vvv --use-ssh-tunnel"

if [[ -n "$HAS_SSH_FEATURES" ]]; then
    # SSH features are available
    CMD+=" --tunnel-api-mode"
fi

if [[ -n "$HAS_PROTO_FEATURES" ]]; then
    # Proto features are available
    CMD+=" --use-protobuf --use-compression"
fi

# Add JSON output flag
CMD+=" --json"

# Print the command
echo "$ $CMD"
echo "=================================================================="

if [[ -z "$HAS_SSH_FEATURES" ]]; then
    echo "NOTE: This binary wasn't compiled with full SSH features support."
fi

if [[ -z "$HAS_PROTO_FEATURES" ]]; then
    echo "NOTE: This binary wasn't compiled with protocol features support."
    echo "The internal protocol defaults will still be used when available."
fi

# Execute the command
eval "$CMD"
TUNNEL_API_EXIT_CODE=$?

echo ""
echo "Exit code: $TUNNEL_API_EXIT_CODE"
echo ""

# Second command: Run without tunnel mode to show the difference
echo "Running without tunnel mode for comparison (uses stdout parsing):"
echo "=================================================================="
echo "$ ./target/release/dedups $SSH_PATH -vvv --json"
echo "=================================================================="

./target/release/dedups "$SSH_PATH" -vvv --json
STDOUT_API_EXIT_CODE=$?

echo ""
echo "Exit code: $STDOUT_API_EXIT_CODE"
echo ""

echo "====================================================================="
echo "  Summary"
echo "====================================================================="
echo ""
echo "The first example used proper tunnel API communication, where:"
echo "1. A SSH tunnel is established to create a direct channel"
echo "2. The remote dedups server is started automatically (--server-mode)"
echo "3. Communication occurs via the tunnel using Protobuf protocol"
echo "4. The server stays alive for the duration of the connection"
echo "5. The server is automatically terminated when the client disconnects"
echo ""
echo "The second example used the traditional stdout parsing approach, where:"
echo "1. Regular SSH command execution is used"
echo "2. Output is parsed from stdout by the client"
echo "3. No long-running server is required"
echo "4. This method is more prone to errors due to SSH output formatting"
echo ""
echo "For reliable communication, especially for SSH operations, you should use:"
RECOMMENDED_FLAGS="--use-ssh-tunnel"
if [[ -n "$HAS_SSH_FEATURES" ]]; then
    RECOMMENDED_FLAGS+=" --tunnel-api-mode"
fi
if [[ -n "$HAS_PROTO_FEATURES" ]]; then
    RECOMMENDED_FLAGS+=" --use-protobuf --use-compression"
fi
echo "  $RECOMMENDED_FLAGS"
echo ""
if [[ -z "$HAS_SSH_FEATURES" || -z "$HAS_PROTO_FEATURES" ]]; then
    echo "To enable all features, compile with: cargo build --release --features ssh,proto"
fi
echo ""

exit 0 