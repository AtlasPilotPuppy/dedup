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

# Check which features are available locally
HAS_SSH_FEATURES=$(./target/release/dedups --help | grep -- "--tunnel-api-mode" || echo "")
HAS_PROTO_FEATURES=$(./target/release/dedups --help | grep -- "--use-protobuf" || echo "")

# Check remote dedups features
echo "Checking remote dedups features..."
# Prioritize ~/.local/bin then search PATH. If not found, try a broader search.
REMOTE_DEDUPS_PATH_CMD="
if [ -x \\"\$HOME/.local/bin/dedups\\" ]; then
    echo \\"\$HOME/.local/bin/dedups\\"
else
    command -v dedups 2>/dev/null
fi
"
REMOTE_DEDUPS_PATH=$(ssh $SSH_HOST "$REMOTE_DEDUPS_PATH_CMD" || echo "")

if [[ -z "$REMOTE_DEDUPS_PATH" ]]; then
    echo "WARNING: dedups not found on remote system using preferred checks (~/.local/bin/dedups or command -v dedups in PATH)."
    echo "Attempting a broader search for dedups on remote (this might take a moment)..."
    # This broader search can be slow and is a last resort.
    REMOTE_DEDUPS_PATH=$(ssh $SSH_HOST "find \\$HOME /usr/local /opt -name dedups -type f -executable -print -quit 2>/dev/null" || echo "")
    if [[ -n "$REMOTE_DEDUPS_PATH" ]]; then
        echo "Dedups found via broader search at: $REMOTE_DEDUPS_PATH (ensure this is the intended instance)"
    else
        echo "CRITICAL WARNING: dedups executable NOT FOUND on remote system ('$SSH_HOST') after all checks."
        echo "Remote operations will likely fail. Please ensure dedups is installed and accessible on the remote system."
        # It might be prudent to exit here if remote dedups is essential for the script's purpose
    fi
fi

if [[ -n "$REMOTE_DEDUPS_PATH" ]]; then
    echo "Remote dedups found at: $REMOTE_DEDUPS_PATH"
    # Use the full path to run dedups
    REMOTE_HAS_SSH=$(ssh $SSH_HOST "\"$REMOTE_DEDUPS_PATH\" --help" 2>/dev/null | grep -- "--tunnel-api-mode" || echo "")
    REMOTE_HAS_PROTO=$(ssh $SSH_HOST "\"$REMOTE_DEDUPS_PATH\" --help" 2>/dev/null | grep -- "--use-protobuf" || echo "")
    
    if [[ -z "$REMOTE_HAS_SSH" ]]; then
        echo "WARNING: Remote dedups does not have SSH features enabled"
    fi
    if [[ -z "$REMOTE_HAS_PROTO" ]]; then
        echo "WARNING: Remote dedups does not have protocol features enabled"
    fi
    if [[ -n "$REMOTE_HAS_SSH" && -n "$REMOTE_HAS_PROTO" ]]; then
        echo "Remote dedups has all required features enabled"
    fi
else
    echo "WARNING: dedups not found on remote system (path determined as empty or check failed)."
    echo "The remote system will likely use fallback mode or fail if the dedups client attempts remote execution."
fi
echo ""

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

# Execute the command and capture output
echo "Establishing SSH tunnel and starting remote server (delegated to dedups client)..."
echo "NOTE: Success depends on the dedups client correctly:"
echo "  1. Using an appropriate remote dedups executable (ideally found at '$REMOTE_DEDUPS_PATH')."
echo "  2. Forcing the remote dedups instance into --server-mode."
echo "  3. Establishing the SSH tunnel for API communication over Protobuf."
OUTPUT=$(eval "$CMD" 2>&1)
TUNNEL_API_EXIT_CODE=$?

# Check for JSON errors first, as dedups might exit 0 even if there's an internal error
JSON_ERROR_MESSAGE=$(echo "$OUTPUT" | grep '{"type":"error","message":')

if [[ $TUNNEL_API_EXIT_CODE -ne 0 ]] || [[ -n "$JSON_ERROR_MESSAGE" ]]; then
    echo "❌ Error reported during dedups execution with tunnel mode."
    if [[ -n "$JSON_ERROR_MESSAGE" ]]; then
        # Try to extract a cleaner error message from JSON if possible
        ERROR_DETAIL=$(echo "$JSON_ERROR_MESSAGE" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p' | head -n 1)
        CODE_DETAIL=$(echo "$JSON_ERROR_MESSAGE" | sed -n 's/.*"code":\([0-9]*\).*/\1/p' | head -n 1)
        echo "Dedups internal error: $ERROR_DETAIL (code: $CODE_DETAIL)"
    fi
    if [[ $TUNNEL_API_EXIT_CODE -ne 0 ]]; then
        echo "Dedups process exit code: $TUNNEL_API_EXIT_CODE"
    fi
    echo ""
    echo "Full output/debug information for tunnel mode (last 20 lines):"
    echo "-------------------------------------------------------------"
    echo "$OUTPUT" | tail -20
elif echo "$OUTPUT" | grep -q "Server communication established"; then
    echo "✅ Server communication established successfully via tunnel API mode."
    echo ""
    echo "Server output details (filtered for 'Server' or 'communication'):"
    echo "-----------------------------------------------------------------"
    echo "$OUTPUT" | grep -E "Server|communication" || echo "(No specific 'Server' or 'communication' lines in output)"
elif echo "$OUTPUT" | grep -q "Using fallback mode"; then
    echo "⚠️  Dedups appears to be using fallback mode."
    echo "This indicates tunnel API mode (--tunnel-api-mode) may not have been fully established or dedups chose not to use it."
    echo ""
    echo "Server connection details (filtered for 'Server', 'connection', 'fallback'):"
    echo "--------------------------------------------------------------------------"
    echo "$OUTPUT" | grep -E "Server|connection|fallback" || echo "(No specific 'Server', 'connection', or 'fallback' lines in output)"
else
    echo "❔ Server communication status unknown for tunnel mode."
    echo "No explicit success, fallback, or JSON error message was detected in the output."
    echo "This could mean tunnel mode did not engage as expected or output is not conforming to expected patterns."
    echo ""
    echo "Full output/debug information for tunnel mode (last 20 lines):"
    echo "-------------------------------------------------------------"
    echo "$OUTPUT" | tail -20
fi

echo ""
echo "Dedups (tunnel mode command) exit code captured by script: $TUNNEL_API_EXIT_CODE"
echo ""

# Second command: Run without tunnel mode to show the difference
echo "Running without tunnel mode for comparison (uses stdout parsing):"
echo "=================================================================="
echo "$ ./target/release/dedups $SSH_PATH -vvv --json"
echo "=================================================================="

OUTPUT_STDOUT=$(./target/release/dedups "$SSH_PATH" -vvv --json 2>&1)
STDOUT_API_EXIT_CODE=$?
JSON_ERROR_STDOUT=$(echo "$OUTPUT_STDOUT" | grep '{"type":"error","message":')

if [[ $STDOUT_API_EXIT_CODE -ne 0 ]] || [[ -n "$JSON_ERROR_STDOUT" ]]; then
    echo "❌ Error reported during dedups execution (stdout parsing mode)."
    if [[ -n "$JSON_ERROR_STDOUT" ]]; then
        ERROR_DETAIL_STDOUT=$(echo "$JSON_ERROR_STDOUT" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p' | head -n 1)
        CODE_DETAIL_STDOUT=$(echo "$JSON_ERROR_STDOUT" | sed -n 's/.*"code":\([0-9]*\).*/\1/p' | head -n 1)
        echo "Dedups internal error: $ERROR_DETAIL_STDOUT (code: $CODE_DETAIL_STDOUT)"
    fi
    if [[ $STDOUT_API_EXIT_CODE -ne 0 ]]; then
        echo "Dedups process exit code: $STDOUT_API_EXIT_CODE"
    fi
    echo ""
    echo "Full output/debug information for stdout mode (last 20 lines):"
    echo "-------------------------------------------------------------"
    echo "$OUTPUT_STDOUT" | tail -20
else
    # Basic check if any output was produced, assuming success if no error and some output
    if [[ -n "$OUTPUT_STDOUT" ]]; then
        echo "✅ Dedups (stdout parsing mode) completed. Output (first 10 lines):"
        echo "$OUTPUT_STDOUT" | head -10
    else
        echo "❔ Dedups (stdout parsing mode) completed with no output and no explicit error."
    fi
fi

echo ""
echo "Dedups (stdout parsing mode command) exit code captured by script: $STDOUT_API_EXIT_CODE"
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