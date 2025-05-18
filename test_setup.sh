#!/bin/bash
set -e

# Base directory for test files
TEST_DIR="$(pwd)/test_files"

# Generate a test SSH key if it doesn't exist
if [ ! -f ~/.ssh/dedups_test_key ]; then
  echo "Generating a new SSH key for testing..."
  ssh-keygen -t rsa -b 4096 -f ~/.ssh/dedups_test_key -N "" -C "dedups-test-key"
  echo "Key generated at ~/.ssh/dedups_test_key"
fi

# Set proper permissions on the SSH key
chmod 600 ~/.ssh/dedups_test_key
chmod 644 ~/.ssh/dedups_test_key.pub

# Remove any existing container
docker rm -f dedups-test-container 2>/dev/null || true

# Build the Docker image if needed
if ! docker images | grep -q dedups-test; then
  echo "Building Docker test image..."
  docker build -t dedups-test -f Dockerfile.test .
fi

# Start the container with the proper volume mounts
echo "Starting Docker container with volume mounts..."
docker run -d --name dedups-test-container \
  -p 2222:22 \
  -v "${TEST_DIR}/bin:/home/testuser/.local/bin" \
  dedups-test

# Wait for SSH to be ready
echo "Waiting for SSH service to start..."
sleep 3

# Copy the SSH public key to the container
echo "Setting up SSH key authentication..."
docker cp ~/.ssh/dedups_test_key.pub dedups-test-container:/tmp/
docker exec dedups-test-container bash -c "cat /tmp/dedups_test_key.pub > /home/testuser/.ssh/authorized_keys"
docker exec dedups-test-container bash -c "chmod 600 /home/testuser/.ssh/authorized_keys && chown testuser:testuser /home/testuser/.ssh/authorized_keys"

# Configure local SSH to use our key
echo "Updating SSH config..."
cat > ~/.ssh/config.new << EOF
# Keep existing config
$(cat ~/.ssh/config 2>/dev/null | grep -v "Host test-dedups" | grep -v "    HostName localhost" | grep -v "    Port 2222" | grep -v "    User testuser" | grep -v "    IdentityFile ~/.ssh/dedups_test_key" || echo "")

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