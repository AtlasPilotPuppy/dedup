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

### Changed
- Enhanced SSH handling with better error messages
- Made SSH tunnel the default for JSON streaming operations 
- Improved error handling and fallback mechanisms

### Fixed
- Fixed JSON streaming issues with remote dedups instances
- Improved handling of JSON parsing from remote systems
- Better error reporting for SSH communication issues
- Fixed buffering problems that could corrupt JSON output
- Enhanced remote dedups detection and communication
- Fixed issues with JSON streaming over SSH when using --json flag
- Fixed permissions handling in SSH tunnel mode

## [0.1.0] - 2023-06-15

### Added
- Initial release
- File deduplication based on content hashing
- Support for various hash algorithms (xxHash, Blake3, SHA1, etc.)
- Interactive TUI mode
- SSH support for remote deduplication 