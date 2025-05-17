#!/bin/bash
set -e

# Configuration
CONTAINER_NAME="dedups-test-container"
SSH_PORT=2222
DATA_DIR="$(pwd)/test_data"
TARGET_DIR="/data"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

# Ensure clean state
cleanup() {
    echo "Cleaning up..."
    docker stop $CONTAINER_NAME 2>/dev/null || true
    docker rm $CONTAINER_NAME 2>/dev/null || true
}

# Create test data directory if it doesn't exist
setup_test_data() {
    mkdir -p "$DATA_DIR/source" "$DATA_DIR/target"
    echo "Test file 1" > "$DATA_DIR/source/file1.txt"
    echo "Test file 2" > "$DATA_DIR/source/file2.txt"
    cp "$DATA_DIR/source/file1.txt" "$DATA_DIR/source/file1_dup.txt"
}

# Build Docker image
build_image() {
    echo "Building test Docker image..."
    
    # Create SSH key if it doesn't exist
    if [ ! -f ~/.ssh/dedups_test_key ]; then
        ssh-keygen -t ed25519 -f ~/.ssh/dedups_test_key -N ""
    fi
    
    # Create a temporary directory for Docker build context
    TMPDIR=$(mktemp -d)
    
    # Copy the public key for the Docker image
    cp ~/.ssh/dedups_test_key.pub $TMPDIR/authorized_keys
    
    # Create a Dockerfile that includes the key
    cat > $TMPDIR/Dockerfile << EOF
FROM ubuntu:22.04

# Install required packages
RUN apt-get update && apt-get install -y \\
    openssh-server \\
    sudo \\
    netcat \\
    coreutils \\
    rsync \\
    && rm -rf /var/lib/apt/lists/*

# Set up SSH server
RUN mkdir /var/run/sshd
RUN sed -i 's/#PermitRootLogin prohibit-password/PermitRootLogin no/' /etc/ssh/sshd_config
RUN sed -i 's/#PubkeyAuthentication yes/PubkeyAuthentication yes/' /etc/ssh/sshd_config
RUN sed -i 's/#PasswordAuthentication yes/PasswordAuthentication no/' /etc/ssh/sshd_config

# Create test user with sudo access
RUN useradd -m -s /bin/bash testuser
RUN echo 'testuser ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers

# Set up SSH for testuser
RUN mkdir -p /home/testuser/.ssh \\
    && mkdir -p /home/testuser/.local/bin

# Copy the pre-generated public key
COPY authorized_keys /home/testuser/.ssh/authorized_keys

# Set proper permissions
RUN chmod 700 /home/testuser/.ssh \\
    && chmod 600 /home/testuser/.ssh/authorized_keys \\
    && chown -R testuser:testuser /home/testuser

# Create test data directory
RUN mkdir -p /data \\
    && chown -R testuser:testuser /data

# Create a mock dedups script for basic testing
RUN echo '#!/bin/bash\\nif [ "\$1" = "--version" ]; then\\n  echo "dedups 0.1.0 (test)"\\nelif [ "\$1" = "--server-mode" ]; then\\n  PORT=\${3:-10000}\\n  nc -l \$PORT\\nelse\\n  echo \"{\\\"type\\\":\\\"result\\\",\\\"payload\\\":\\\"{\\\\\\\"duplicate_count\\\\\\\":1,\\\\\\\"total_files\\\\\\\":3,\\\\\\\"total_bytes\\\\\\\":50,\\\\\\\"duplicate_bytes\\\\\\\":20,\\\\\\\"elapsed_seconds\\\\\\\":0.1}\\\"}\"\\nfi' > /home/testuser/.local/bin/dedups \\
    && chmod +x /home/testuser/.local/bin/dedups

# Start SSH server
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D"]
EOF
    
    # Build the Docker image
    docker build -t dedups-test -f $TMPDIR/Dockerfile $TMPDIR
    
    # Clean up temporary directory
    rm -rf $TMPDIR
}

# Start test container
start_container() {
    echo "Starting test container..."
    docker run -d --name $CONTAINER_NAME -p $SSH_PORT:22 -v "$DATA_DIR:$TARGET_DIR" dedups-test
    
    # Wait for SSH to be available
    echo -n "Waiting for SSH to be ready "
    for i in {1..10}; do
        if ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost echo "SSH is ready" &>/dev/null; then
            echo -e "\nSSH is ready!"
            return 0
        fi
        echo -n "."
        sleep 1
    done
    
    echo -e "\nSSH connection timed out!"
    return 1
}

# Test basic SSH connectivity
test_ssh_connectivity() {
    echo "Testing basic SSH connectivity..."
    if ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost echo "SSH test succeeded" | grep -q "SSH test succeeded"; then
        echo -e "${GREEN}✓ SSH connectivity test passed${NC}"
    else
        echo -e "${RED}✗ SSH connectivity test failed${NC}"
        return 1
    fi
}

# Test dedups version over SSH
test_dedups_version() {
    echo "Testing dedups version command..."
    if ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "~/.local/bin/dedups --version" | grep -q "dedups"; then
        echo -e "${GREEN}✓ Dedups version test passed${NC}"
    else
        echo -e "${RED}✗ Dedups version test failed${NC}"
        return 1
    fi
}

# Test direct tunnel connection
test_direct_tunnel() {
    echo "Testing direct tunnel connection..."
    
    # Start netcat server on remote host
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT -f testuser@localhost "nc -l 10000 > /tmp/tunnel_test.txt" 
    sleep 1
    
    # Set up SSH tunnel
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT -L 10000:localhost:10000 -f testuser@localhost sleep 5
    sleep 1
    
    # Send test data through tunnel
    echo "Test data through tunnel" | nc localhost 10000
    sleep 1
    
    # Check if data was received
    if ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "cat /tmp/tunnel_test.txt" | grep -q "Test data through tunnel"; then
        echo -e "${GREEN}✓ Direct tunnel test passed${NC}"
    else
        echo -e "${RED}✗ Direct tunnel test failed${NC}"
        return 1
    fi
}

# Create files for test on remote host
test_setup_files() {
    echo "Setting up test files..."
    
    # Upload files directly to the container
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "mkdir -p /data/test"
    echo "Test file 1" | ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "cat > /data/test/file1.txt"
    echo "Test file 2" | ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "cat > /data/test/file2.txt"
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "cp /data/test/file1.txt /data/test/file1_dup.txt"
    
    echo "Test files created in container at /data/test"
}

# Test dedups JSON streaming with direct SSH command
test_dedups_json_direct() {
    echo "Testing JSON with direct ssh command..."
    
    RESULT=$(ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "~/.local/bin/dedups /data/test --json")
    echo "$RESULT"
    
    if echo "$RESULT" | grep -q '"type":"result"'; then
        echo -e "${GREEN}✓ Direct SSH JSON test passed${NC}"
        return 0
    else  
        echo -e "${RED}✗ Direct SSH JSON test failed${NC}"
        return 1
    fi
}

# Test JSON streaming with SSH tunnel
test_ssh_json_tunnel() {
    echo "Testing JSON streaming through SSH tunnel..."
    
    # Create temporary directory for socket
    TUNNEL_PORT=10000
    
    # Start netcat server on remote host with timeout
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "~/.local/bin/dedups /data/test --json > /tmp/output.json" 
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT testuser@localhost "cat /tmp/output.json | nc -l $TUNNEL_PORT" &
    SERVER_PID=$!
    
    # Give the remote server time to start
    sleep 2
    
    # Set up SSH tunnel
    ssh -i ~/.ssh/dedups_test_key -o StrictHostKeyChecking=no -p $SSH_PORT -L $TUNNEL_PORT:localhost:$TUNNEL_PORT -f testuser@localhost sleep 10
    
    # Check if tunnel is established - use direct test
    echo "Waiting for tunnel..."
    sleep 3
    
    # Read from the tunnel
    RESULT=$(echo "" | nc -w 5 localhost $TUNNEL_PORT)
    echo "Tunnel result: $RESULT"
    
    # Clean up
    kill $SERVER_PID 2>/dev/null || true
    
    # Check if we got valid JSON
    if echo "$RESULT" | grep -q '"type":"result"'; then
        echo -e "${GREEN}✓ SSH tunnel JSON test passed${NC}"
        return 0
    else  
        echo -e "${RED}✗ SSH tunnel JSON test failed${NC}"
        return 1
    fi
}

# Run all tests
run_tests() {
    echo "Running tests..."
    test_ssh_connectivity
    test_dedups_version
    test_direct_tunnel
    test_setup_files
    test_dedups_json_direct
    test_ssh_json_tunnel
}

# Main function
main() {
    # Register cleanup on script exit
    trap cleanup EXIT
    
    # Clean up any previous containers
    cleanup
    
    # Remove previous localhost key for port 2222 to avoid warnings
    ssh-keygen -R [localhost]:$SSH_PORT 2>/dev/null || true
    
    # Set up test data
    setup_test_data
    
    # Build Docker image
    build_image
    
    # Start container
    start_container
    
    # Run tests
    run_tests
    
    echo -e "\n${GREEN}All tests completed successfully!${NC}"
}

# Run main function
main 