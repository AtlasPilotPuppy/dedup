# SSH JSON Streaming Implementation

## Overview
This document describes the implementation of reliable JSON streaming over SSH connections in the dedups tool.

## Key Components

### 1. Client-Server Architecture
- **Protocol Module** (`protocol.rs`): Defines message types and handlers for client-server communication
- **Server Module** (`server.rs`): Implements a server for handling remote dedups commands
- **Client Module** (`client.rs`): Implements a client for connecting to dedups servers

### 2. SSH Tunnel Implementation
- Uses port forwarding to create a reliable bidirectional communication channel
- Automatically falls back to standard SSH when needed
- Leverages netcat (nc) on remote systems when available

### 3. Connection Options
- **SSH_COMMAND Environment Variable**: Override the SSH command implementation
- **SSH_CONFIG_FILE Environment Variable**: Specify a custom SSH config file
- **CLI Flags**: `--use-ssh-tunnel` to control tunneling behavior

## Communication Flow

1. **Remote Detection**:
   - Check if the target path is remote (SSH URL format)
   - Connect to remote host and verify dedups availability

2. **SSH Tunnel Setup**:
   - Find available local port
   - Start server process on remote system
   - Establish SSH tunnel to the remote port

3. **JSON Streaming**:
   - Send command to server through tunnel
   - Receive streaming JSON responses
   - Process structured data for progress updates and results

## Configuration

Configure SSH tunnel behavior in your `.deduprc` file:
```toml
[ssh]
use_ssh_tunnel = true  # Enable/disable SSH tunneling (default: true)
```

Or via command line:
```bash
dedups --use-ssh-tunnel ssh:user@host:/path --json
```

## Testing

Comprehensive Docker-based tests ensure reliable functionality:
- Direct SSH command tests
- Socket tunnel tests
- Error handling and fallback tests

The test harness in `test_ssh_tunnel.sh` provides a complete testing environment.

## Troubleshooting

### Requirements
- SSH access to the remote system
- netcat (`nc`) on the remote system for optimal performance
- Standard buffering utilities (`stdbuf`) for fallback mode

### Debugging
- Set `RUST_LOG=debug` for detailed logs
- Use `--verbose` flag to increase output detail

### Common Issues
- Permissions problems: Check SSH key access
- Missing tools: Install netcat on remote system
- Port conflicts: Close any applications using the same ports 