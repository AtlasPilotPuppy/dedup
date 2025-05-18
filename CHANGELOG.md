# Changelog

## [Unreleased]

### Added
- Advanced SSH tunneling for reliable JSON streaming over remote connections
- Socket-based communication protocol for improved JSON data transmission 
- Automatic fallback mechanisms when network tools like `netcat` are unavailable
- Support for unbuffered output streams using `stdbuf` or `unbuffer` when available
- Comprehensive structured JSON output with detailed progress information
- Command-line flag `--use-ssh-tunnel` to control tunnel usage (defaults to on)
- Improved SSH JSON streaming support via TCP socket tunneling
- Server mode for remote execution with reliable JSON streaming
- Support for SSH command override via SSH_COMMAND environment variable
- Detailed progress reporting over SSH tunnel
- Docker-based integration tests for SSH functionality
- SSH tunnel API communication mode with Protobuf and compression support
- Example script for demonstrating SSH API modes (scripts/ssh_api_example.sh)
- Better protocol defaults for reliable SSH communication
- Improved server startup and client connection handling

### Changed
- Enhanced SSH handling with better error messages
- Made SSH tunnel the default for JSON streaming operations 
- Improved error handling and fallback mechanisms
- Made tunnel-api-mode enabled by default when using SSH tunnels
- Set Protobuf and compression to be enabled by default for protocol communication
- Enhanced error handling and diagnostic output for SSH connections
- Updated documentation with SSH communication best practices

### Fixed
- Fixed JSON streaming issues with remote dedups instances
- Improved handling of JSON parsing from remote systems
- Better error reporting for SSH communication issues
- Fixed buffering problems that could corrupt JSON output
- Enhanced remote dedups detection and communication
- Fixed issues with JSON streaming over SSH when using --json flag
- Fixed permissions handling in SSH tunnel mode
- Issue where SSH communication was falling back to stdout parsing instead of using API tunnel
- Connection stability problems in SSH tunneling
- Made the SSH protocol detection more robust to handle missing feature flags

## [0.1.0] - 2023-06-15

### Added
- Initial release
- File deduplication based on content hashing
- Support for various hash algorithms (xxHash, Blake3, SHA1, etc.)
- Interactive TUI mode
- SSH support for remote deduplication 