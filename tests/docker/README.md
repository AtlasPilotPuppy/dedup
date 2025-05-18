# Docker SSH API Tests

These tests verify the SSH API communication behavior in the dedups application using Docker containers.

## What These Tests Verify

1. API mode communication: Verifies that when `--tunnel-api-mode` is enabled, the client communicates with the server over a dedicated API protocol rather than parsing stdout.
2. Server lifecycle: Confirms that the server process stays alive during SSH tunnel communication sessions and cleanly terminates when the client disconnects.
3. Comparison with non-API mode: Shows the different behavior when not using API mode.

## Prerequisites

- Docker and Docker Compose must be installed
- You must have sufficient permissions to build Docker images and run containers

## Running the Tests

You can run the tests in two ways:

### Using the script

```bash
./run_docker_tests.sh
```

### Using Cargo (with ignored tests)

```bash
cargo test --test ssh_docker_tests test_ssh_api_communication -- --ignored --nocapture
```

## Test Details

The tests are structured as follows:

1. **API Communication Test**: Verifies that when tunnel API mode is enabled, proper SSH tunnels are established and the server process is launched on the remote machine.

2. **Long-running Server Test**: Ensures that for operations that take longer, the server remains alive throughout the session and is properly cleaned up afterward.

3. **Non-API Mode Test**: For comparison, shows that in non-API mode, no server process is launched.

## Troubleshooting

If the tests fail:
- Check the Docker container logs for detailed information
- Verify that the server container is properly starting the SSH daemon
- Check network connectivity between containers
- Ensure the dedups binary is correctly built with SSH and protobuf features 