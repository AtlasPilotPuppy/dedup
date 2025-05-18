#!/bin/bash
set -e

# Configuration
TEST_ENV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SSH_KEY_PATH="$TEST_ENV_DIR/ssh/test_key"
SSH_PORT=2222

# Step 1: Set up SSH keys if they don't exist
setup_ssh_keys() {
    echo "Setting up SSH keys..."
    mkdir -p "$TEST_ENV_DIR/ssh"
    if [ ! -f "$SSH_KEY_PATH" ]; then
        ssh-keygen -t rsa -b 4096 -f "$SSH_KEY_PATH" -N "" -C "dedups-test-key"
    fi
    chmod 600 "$SSH_KEY_PATH"
    chmod 644 "${SSH_KEY_PATH}.pub"
}

# Step 2: Prepare test data directory
prepare_test_data() {
    echo "Preparing test data..."
    mkdir -p "$TEST_ENV_DIR/test_data"
    # Copy demo directory for testing if it exists
    if [ -d "../demo" ]; then
        echo "Copying demo files..."
        cp -r ../demo/* "$TEST_ENV_DIR/test_data/"
    else
        echo "Creating test files..."
        # Create some test files if demo directory doesn't exist
        dd if=/dev/urandom of="$TEST_ENV_DIR/test_data/test1.bin" bs=1M count=1
        cp "$TEST_ENV_DIR/test_data/test1.bin" "$TEST_ENV_DIR/test_data/test1_duplicate.bin"
        dd if=/dev/urandom of="$TEST_ENV_DIR/test_data/test2.bin" bs=1M count=1
    fi
    chmod -R 755 "$TEST_ENV_DIR/test_data"
}

# Step 3: Make scripts executable
prepare_scripts() {
    echo "Preparing scripts..."
    chmod +x "$TEST_ENV_DIR/bin/dedups"
}

# Step 4: Build and start Docker container
setup_container() {
    echo "Setting up Docker container..."
    # Remove existing container if it exists
    docker rm -f dedups-test-container 2>/dev/null || true
    
    # Build the container if needed
    if ! docker images | grep -q dedups-test; then
        docker build -t dedups-test -f ../Dockerfile.test .
    fi

    # Create SSH directory structure in container
    mkdir -p "$TEST_ENV_DIR/container_home/.ssh"
    cp "${SSH_KEY_PATH}.pub" "$TEST_ENV_DIR/container_home/.ssh/authorized_keys"
    chmod 700 "$TEST_ENV_DIR/container_home/.ssh"
    chmod 600 "$TEST_ENV_DIR/container_home/.ssh/authorized_keys"
    
    # Start container with volume mounts
    docker run -d --name dedups-test-container \
        -p $SSH_PORT:22 \
        -v "$TEST_ENV_DIR/container_home/.ssh:/home/testuser/.ssh" \
        -v "$TEST_ENV_DIR/bin:/home/testuser/.local/bin:ro" \
        -v "$TEST_ENV_DIR/test_data:/home/testuser/test_data" \
        dedups-test

    # Wait for container to be ready
    echo "Waiting for container to be ready..."
    sleep 3
}

# Step 5: Configure local SSH
configure_ssh() {
    echo "Configuring SSH..."
    # Remove existing test-dedups host from SSH config
    sed -i.bak '/^Host test-dedups/,/^$/ d' ~/.ssh/config 2>/dev/null || true
    
    # Add new configuration
    cat >> ~/.ssh/config << EOF

Host test-dedups
    HostName localhost
    Port $SSH_PORT
    User testuser
    IdentityFile $SSH_KEY_PATH
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
EOF
    chmod 600 ~/.ssh/config
    
    # Clear any existing known hosts entry
    ssh-keygen -f ~/.ssh/known_hosts -R "[localhost]:$SSH_PORT" 2>/dev/null || true
}

# Step 6: Test SSH connection and environment
test_connection() {
    echo "Testing SSH connection..."
    sleep 3  # Wait for SSH to be ready
    if ! ssh test-dedups echo "SSH connection successful"; then
        echo "SSH connection test failed"
        exit 1
    fi
    echo "SSH connection test passed"
    
    echo "Testing dedups script..."
    if ! ssh test-dedups /home/testuser/.local/bin/dedups --version; then
        echo "Dedups script test failed"
        exit 1
    fi
    echo "Dedups script test passed"

    echo "Testing file system access..."
    if ! ssh test-dedups "ls -la /home/testuser/test_data && echo 'test' > /home/testuser/test_data/write_test.txt"; then
        echo "File system access test failed"
        exit 1
    fi
    echo "File system access test passed"
}

# Main execution
echo "Setting up test environment..."
setup_ssh_keys
prepare_test_data
prepare_scripts
setup_container
configure_ssh
test_connection

echo "Test environment setup complete!"
echo "You can now run the tests with: cargo test --features ssh --test ssh_integration_tests -- --ignored" 