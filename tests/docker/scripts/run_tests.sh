#!/bin/bash
set -e

# Wait for SSH server to be ready
echo "Waiting for SSH server to be ready..."
for i in {1..30}; do
  if nc -z ssh-server 22; then
    echo "SSH server is ready!"
    break
  fi
  echo "Waiting for SSH server (attempt $i)..."
  sleep 1
done

# Make sure the SSH server is fully operational
sleep 2

# Set up known_hosts to avoid prompt
mkdir -p ~/.ssh
ssh-keyscan -H ssh-server >> ~/.ssh/known_hosts 2>/dev/null
chmod 600 ~/.ssh/known_hosts

# Test SSH connection
echo "Testing SSH connection..."
ssh -o ConnectTimeout=5 testuser@ssh-server "echo SSH connection successful"

# Generate test files on the server
echo "Generating test files on server..."
ssh testuser@ssh-server "mkdir -p /home/testuser/test_files"
for i in {1..5}; do
  ssh testuser@ssh-server "dd if=/dev/urandom of=/home/testuser/test_files/file$i.dat bs=1K count=$i 2>/dev/null"
done

# Create duplicate files
ssh testuser@ssh-server "cp /home/testuser/test_files/file1.dat /home/testuser/test_files/file1_dup.dat"
ssh testuser@ssh-server "cp /home/testuser/test_files/file2.dat /home/testuser/test_files/file2_dup.dat"

# ---------------------------------
# Test 1: Verify API-based communication
# ---------------------------------
echo "Starting Test 1: API Communication Test"

# First, verify server process isn't already running
echo "Checking for existing server processes..."
server_before=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_before" -ne "0" ]; then
  echo "ERROR: Server processes already running before test: $server_before"
  exit 1
fi

# Run dedups with tunnel mode
echo "Running dedups with tunnel API mode..."
/app/target/debug/dedups ssh:testuser@ssh-server:/home/testuser/test_files --json --tunnel-api-mode --verbose > /tmp/test_output.log 2>&1 &
DEDUPS_PID=$!

# Give it time to establish connection
sleep 5

# Check if tunnel was established
echo "Checking for SSH tunnel..."
tunnel_count=$(ssh testuser@ssh-server "ps aux | grep ssh | grep '\-L' | grep -v grep | wc -l")
if [ "$tunnel_count" -eq "0" ]; then
  echo "ERROR: No SSH tunnel established"
  exit 1
else
  echo "SUCCESS: SSH tunnel established"
fi

# Check if server process was started and is still running
echo "Checking for server process..."
server_count=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_count" -eq "0" ]; then
  echo "ERROR: No server process found"
  ssh testuser@ssh-server "ps aux"
  exit 1
else
  echo "SUCCESS: Server is running ($server_count processes)"
fi

# Wait for process to finish
echo "Waiting for dedups to finish..."
wait $DEDUPS_PID || true

# Check if output contains JSON (for proper API communication)
if grep -q "\"type\":\"result\"" /tmp/test_output.log; then
  echo "SUCCESS: Found JSON result in output"
else
  echo "ERROR: No JSON result found in output"
  cat /tmp/test_output.log
  exit 1
fi

# Verify server process is stopped after client completes
sleep 2
server_after=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_after" -ne "0" ]; then
  echo "ERROR: Server processes still running after client exit: $server_after"
  ssh testuser@ssh-server "ps aux | grep dedups"
  exit 1
else
  echo "SUCCESS: Server processes cleaned up properly"
fi

# ---------------------------------
# Test 2: Verify long-running server session
# ---------------------------------
echo "Starting Test 2: Long-running Server Test"

# Run a long operation that requires the server to stay alive
echo "Running long operation..."
/app/target/debug/dedups ssh:testuser@ssh-server:/home/testuser/test_files --algorithm blake3 --json --tunnel-api-mode --verbose > /tmp/long_operation.log 2>&1 &
DEDUPS_PID=$!

# Give it time to establish connection
sleep 5

# Check server is running
server_count=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_count" -eq "0" ]; then
  echo "ERROR: No server process found for long operation"
  exit 1
else
  echo "SUCCESS: Server is running for long operation"
fi

# Check for open port connection
port_connections=$(ssh testuser@ssh-server "netstat -tuln | grep 29875 | wc -l" || echo "0")
if [ "$port_connections" -eq "0" ]; then
  echo "ERROR: No port connection found for API communication"
  ssh testuser@ssh-server "netstat -tuln"
  exit 1
else
  echo "SUCCESS: Port connection established for API communication"
fi

# Wait for process to finish
echo "Waiting for long operation to finish..."
wait $DEDUPS_PID || true

# Verify server process is stopped after client completes
sleep 2
server_after=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_after" -ne "0" ]; then
  echo "ERROR: Server processes still running after long operation: $server_after"
  exit 1
else
  echo "SUCCESS: Server processes cleaned up after long operation"
fi

# ---------------------------------
# Test 3: Verify non-API mode shows different behavior
# ---------------------------------
echo "Starting Test 3: Non-API Mode Test (for comparison)"

# Run dedups without tunnel mode
echo "Running dedups without tunnel API mode..."
/app/target/debug/dedups ssh:testuser@ssh-server:/home/testuser/test_files --json --use-ssh-tunnel=false > /tmp/non_api_output.log 2>&1

# Check for server processes - should be none in non-API mode
server_count=$(ssh testuser@ssh-server "pgrep -f 'dedups.*server-mode' | wc -l" || echo "0")
if [ "$server_count" -ne "0" ]; then
  echo "ERROR: Found server processes in non-API mode: $server_count"
  exit 1
else
  echo "SUCCESS: No server processes in non-API mode (expected)"
fi

# ---------------------------------
# Summary
# ---------------------------------
echo "All tests completed successfully!"

# Exit with success
exit 0 