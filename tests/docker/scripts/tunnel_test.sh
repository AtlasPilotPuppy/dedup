#!/bin/bash
set -e

# Function to display section headers
section() {
  echo ""
  echo "======================================================================================"
  echo "  $1"
  echo "======================================================================================"
  echo ""
}

# Wait for SSH server to be ready
section "Setting up test environment"
echo "Waiting for SSH server to be ready..."
for i in {1..30}; do
  if nc -z ssh-server 22; then
    echo "SSH server is ready!"
    break
  fi
  echo "Waiting for SSH server (attempt $i)..."
  sleep 1
done

# Set up SSH environment
mkdir -p ~/.ssh
ssh-keyscan -H ssh-server >> ~/.ssh/known_hosts 2>/dev/null
chmod 600 ~/.ssh/known_hosts

# Test SSH connection
ssh -o ConnectTimeout=5 testuser@ssh-server "echo SSH connection successful"

# Create test files
section "Creating test data"
ssh testuser@ssh-server "mkdir -p /home/testuser/test_files"
for i in {1..5}; do
  ssh testuser@ssh-server "dd if=/dev/urandom of=/home/testuser/test_files/file$i.dat bs=1K count=$i 2>/dev/null"
done
ssh testuser@ssh-server "cp /home/testuser/test_files/file1.dat /home/testuser/test_files/file1_dup.dat"

# Verify no dedups server processes are running
section "Verifying clean environment"
ssh testuser@ssh-server "pgrep -f dedups || echo 'No dedups processes running'"

# Capture network state before test
section "Capturing baseline network state"
ssh testuser@ssh-server "netstat -tuln > /tmp/netstat_before.log"
ssh testuser@ssh-server "cat /tmp/netstat_before.log"

# Run a dedups command with tunnel API mode and capture traffic
section "Running dedups with tunnel API mode"
echo "Starting packet capture on server..."
ssh testuser@ssh-server "tcpdump -i lo -w /tmp/tunnel_test.pcap port 29875 &" 
TCPDUMP_PID=$(ssh testuser@ssh-server "echo $!")

# Run dedups with tunnel mode and with elevated logging
section "Running dedups command with API tunnel"
export RUST_LOG=dedups=trace,debug
/app/target/debug/dedups ssh:testuser@ssh-server:/home/testuser/test_files --json --tunnel-api-mode > /tmp/tunnel_output.log 2>&1 &
DEDUPS_PID=$!

# Give it time to establish connection
sleep 2

# Check if a tunnel was established
section "Verifying SSH tunnel"
ssh testuser@ssh-server "ps aux | grep ssh"
TUNNEL_COUNT=$(ssh testuser@ssh-server "ps aux | grep 'ssh.*-L.*localhost' | grep -v grep | wc -l")
if [ "$TUNNEL_COUNT" -eq "0" ]; then
  echo "ERROR: No SSH tunnel established. API communication will not work."
  exit 1
else
  echo "SUCCESS: SSH tunnel established with port forwarding"
  ssh testuser@ssh-server "ps aux | grep 'ssh.*-L' | grep -v grep"
fi

# Check if server process was started
section "Verifying server process"
SERVER_COUNT=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l")
if [ "$SERVER_COUNT" -eq "0" ]; then
  echo "ERROR: No server process found. Remote API endpoint not available."
  exit 1
else
  echo "SUCCESS: Server is running ($SERVER_COUNT processes)"
  ssh testuser@ssh-server "ps aux | grep dedups | grep -v grep"
fi

# Check for port binding
section "Verifying port binding"
ssh testuser@ssh-server "netstat -tuln | grep 29875"
PORT_CONNECTIONS=$(ssh testuser@ssh-server "netstat -tuln | grep 29875 | wc -l")
if [ "$PORT_CONNECTIONS" -eq "0" ]; then
  echo "ERROR: No port binding found for API communication"
  exit 1
else
  echo "SUCCESS: Port binding established for API communication"
fi

# Wait for dedups to finish
echo "Waiting for dedups to finish..."
wait $DEDUPS_PID || true

# Stop packet capture
ssh testuser@ssh-server "pkill -f tcpdump" || true

# Analyze the traffic capture
section "Analyzing network traffic"
ssh testuser@ssh-server "tcpdump -r /tmp/tunnel_test.pcap -A | grep -B2 -A2 'type' | head -20"

# Check for JSON messages in the capture
JSON_PATTERNS=$(ssh testuser@ssh-server "tcpdump -r /tmp/tunnel_test.pcap -A | grep -c '\"type\":'")
if [ "$JSON_PATTERNS" -gt "0" ]; then
  echo "SUCCESS: Found JSON protocol messages in network traffic"
else
  echo "ERROR: No JSON protocol messages found in network traffic"
  exit 1
fi

# Check if the output contains proper structured JSON output
section "Verifying client output"
grep -A2 "type" /tmp/tunnel_output.log | head -10

if grep -q "\"type\":\"result\"" /tmp/tunnel_output.log; then
  echo "SUCCESS: Found JSON result in output (proper protocol communication)"
else
  echo "ERROR: No JSON result found in output"
  exit 1
fi

# Check if server process is stopped after client completes 
section "Verifying server cleanup"
sleep 2
SERVER_AFTER=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l")
if [ "$SERVER_AFTER" -ne "0" ]; then
  echo "ERROR: Server processes still running after client exit: $SERVER_AFTER"
  exit 1
else
  echo "SUCCESS: Server processes cleaned up properly"
fi

# Run dedups in non-tunnel mode for comparison
section "Comparison test: Running without API tunnel"
echo "Running dedups without tunnel API mode..."
/app/target/debug/dedups ssh:testuser@ssh-server:/home/testuser/test_files --json --use-ssh-tunnel=false > /tmp/non_tunnel_output.log 2>&1

# Check for server process - should be none in non-tunnel mode
SERVER_COUNT=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l")
if [ "$SERVER_COUNT" -ne "0" ]; then
  echo "ERROR: Found server processes in non-API mode: $SERVER_COUNT"
  exit 1
else
  echo "SUCCESS: No server processes in non-API mode (expected)"
fi

section "TEST RESULTS"
echo "✅ Test PASSED: SSH tunnel established and used for API communication"
echo "✅ Server process started and bound to port"
echo "✅ JSON protocol messages detected in network traffic"
echo "✅ Server properly cleaned up after client disconnect"
echo ""
echo "The dedups application is correctly using the tunnel API for communication"
echo "rather than relying on parsing stdout over SSH."

exit 0 