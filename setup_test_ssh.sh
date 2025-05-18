#!/bin/bash

set -e

# Generate a test SSH key if it doesn't exist
if [ ! -f ~/.ssh/dedups_test_key ]; then
  echo "Generating a new SSH key for testing..."
  ssh-keygen -t rsa -b 4096 -f ~/.ssh/dedups_test_key -N "" -C "dedups-test-key"
  echo "Key generated at ~/.ssh/dedups_test_key"
fi

# Make sure SSH key has proper permissions
chmod 600 ~/.ssh/dedups_test_key
chmod 644 ~/.ssh/dedups_test_key.pub

# Create dedicated directory for dedups test files if it doesn't exist
TEST_DIR="$HOME/dedups_test_dir"
mkdir -p "$TEST_DIR/bin"

# Create a mock dedups script for testing
cat > "$TEST_DIR/bin/dedups" << 'EOF'
#!/bin/bash
echo "dedups v0.1.0-test"
echo "Running command: $@"
echo "Success"
EOF
chmod +x "$TEST_DIR/bin/dedups"

# Check if container is already running
if docker ps | grep -q dedups-test-container; then
  echo "Container already running, skipping build..."
else
  # Clean up any existing stopped container
  docker rm -f dedups-test-container 2>/dev/null || true
  
  # Build the Docker image if it doesn't exist
  if ! docker images | grep -q dedups-test; then
    echo "Building Docker container image..."
    docker build -t dedups-test -f Dockerfile.test .
  fi
  
  # Start the container with volume mount
  echo "Starting Docker container with volume mounts..."
  docker run -d --name dedups-test-container \
    -p 2222:22 \
    -v "$TEST_DIR/bin:/mnt/dedups_bin" \
    dedups-test
  
  # Wait for SSH to be ready
  echo "Waiting for SSH service to start..."
  sleep 3
fi

# Copy the SSH public key to the container
echo "Copying SSH public key to container..."
docker cp ~/.ssh/dedups_test_key.pub dedups-test-container:/tmp/
docker exec dedups-test-container bash -c "cat /tmp/dedups_test_key.pub > /home/testuser/.ssh/authorized_keys"
docker exec dedups-test-container bash -c "chmod 600 /home/testuser/.ssh/authorized_keys && chown testuser:testuser /home/testuser/.ssh/authorized_keys"

# Create a symbolic link to the mounted dedups script
docker exec dedups-test-container bash -c "mkdir -p /home/testuser/.local/bin"
docker exec dedups-test-container bash -c "ln -sf /mnt/dedups_bin/dedups /home/testuser/.local/bin/dedups"
docker exec dedups-test-container bash -c "chown -R testuser:testuser /home/testuser/.local/bin"
docker exec dedups-test-container bash -c "chmod 755 /home/testuser/.local/bin"

# Configure local SSH to use this key for the test-dedups host
echo "Updating SSH config..."
cat > ~/.ssh/config.new << EOF
# Keep existing config
$(cat ~/.ssh/config 2>/dev/null | grep -v "Host test-dedups" | grep -v "HostName localhost" | grep -v "Port 2222" | grep -v "User testuser" | grep -v "IdentityFile ~/.ssh/dedups_test_key" || echo "")

# Dedups test container configuration
Host test-dedups
    HostName localhost
    Port 2222
    User testuser
    IdentityFile ~/.ssh/dedups_test_key
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
EOF

mv ~/.ssh/config.new ~/.ssh/config
chmod 600 ~/.ssh/config

# Clear any existing known hosts entry for localhost:2222
ssh-keygen -f ~/.ssh/known_hosts -R "[localhost]:2222" 2>/dev/null || true

# Test SSH connection
echo "Testing SSH connection..."
ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p 2222 testuser@localhost echo "SSH connection successful"

# Verify mock dedups script is working
echo "Verifying mock dedups script in container..."
ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p 2222 testuser@localhost /home/testuser/.local/bin/dedups --version

echo "Setup complete. You can now run tests with: cargo test --features ssh --test ssh_integration_tests -- --ignored" 