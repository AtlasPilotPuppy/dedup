#!/bin/bash

# Build the test container
docker build -t dedups-test -f Dockerfile.test .

# Run the container
docker run -d --name dedups-test-container -p 2222:22 dedups-test

# Wait for SSH to be ready
echo "Waiting for SSH to be ready..."
sleep 5

# Generate SSH key if it doesn't exist
if [ ! -f ~/.ssh/id_rsa ]; then
    ssh-keygen -t rsa -N "" -f ~/.ssh/id_rsa
fi

# Add test host to SSH config
cat >> ~/.ssh/config << EOF

Host test-dedups
    HostName localhost
    Port 2222
    User testuser
    StrictHostKeyChecking no
    PasswordAuthentication yes
EOF

# Copy SSH key to container
sshpass -p 'testpassword' ssh-copy-id -p 2222 testuser@localhost

echo "Test container is ready!"
echo "You can now run the tests with: cargo test -- --ignored" 