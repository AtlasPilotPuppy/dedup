FROM rust:1.71-slim

# Install SSH server and utilities
RUN apt-get update && apt-get install -y \
    openssh-server \
    rsync \
    netcat-openbsd \
    net-tools \
    coreutils \
    expect \
    procps \
    lsof \
    --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

# Setup SSH server
RUN mkdir /var/run/sshd
RUN sed -i 's/#PermitRootLogin prohibit-password/PermitRootLogin yes/' /etc/ssh/sshd_config
RUN sed -i 's/#PasswordAuthentication yes/PasswordAuthentication yes/' /etc/ssh/sshd_config

# Setup test user
RUN useradd -m -s /bin/bash testuser
RUN echo "testuser:testpassword" | chpasswd

# Generate SSH keys for passwordless authentication
RUN mkdir -p /home/testuser/.ssh
RUN ssh-keygen -t rsa -f /home/testuser/.ssh/id_rsa -N ""
RUN cp /home/testuser/.ssh/id_rsa.pub /home/testuser/.ssh/authorized_keys
RUN chown -R testuser:testuser /home/testuser/.ssh
RUN chmod 700 /home/testuser/.ssh
RUN chmod 600 /home/testuser/.ssh/authorized_keys

# Copy project to container
WORKDIR /app
COPY . .

# Build the project with all features
RUN cargo build --features "ssh proto"

# Copy the binary to testuser's home
RUN cp target/debug/dedups /home/testuser/

# Expose SSH port
EXPOSE 22

# Entry point to start SSH server
CMD ["/usr/sbin/sshd", "-D"] 