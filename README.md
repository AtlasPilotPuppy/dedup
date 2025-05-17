# dedups

A high-performance duplicate file finder and manager written in Rust. `dedups` efficiently identifies duplicate files using parallel processing and provides both a command-line interface and an interactive Terminal User Interface (TUI) for managing the results.

[![Tests](https://github.com/AtlasPilotPuppy/dedup/actions/workflows/tests.yml/badge.svg)](https://github.com/AtlasPilotPuppy/dedup/actions/workflows/tests.yml)

## Features

- **High Performance**: Uses multi-threading with Rayon for parallel hash calculation
- **Multiple Hash Algorithms**: Choose between MD5, SHA1, SHA256, Blake3, xxHash (default), GxHash, FNV1a, or CRC32
- **Interactive TUI**: Visually inspect and manage duplicate files
- **Media Deduplication**: Identify similar media files that differ in format, resolution, or quality
- **File Cache**: Store and reuse file hash values to speed up repeated scans of unchanged files
- **Selection Strategies**: Various automated selection strategies for keeping/removing duplicates
  - Shortest path: Keep files with the shortest paths
  - Longest path: Keep files with the longest paths
  - Newest modified: Keep the most recently modified files
  - Oldest modified: Keep the oldest files
- **Operations**: Delete duplicates or move them to a specified location
- **Filtering**: Include/exclude files by glob patterns
- **Output Formats**: Save duplicate file information as JSON or TOML
- **Configurable**: Adjust thread count, verbosity, sorting options, and more
- **Configuration File**: Use a custom configuration file to set defaults
- **Dry Run Mode**: Simulate operations without making actual changes
- **Planned Integration**: Future integration with rclone for cloud storage deduplication

## Installation

### Quick Install (Bash)

```bash
# Download and install the latest release
curl -sSL https://raw.githubusercontent.com/AtlasPilotPuppy/dedup/main/install.sh | bash
```

If you need SSH/remote file system support, use:

```bash
# Install with SSH support
curl -sSL https://raw.githubusercontent.com/AtlasPilotPuppy/dedup/main/install.sh | bash -s -- --ssh
```

Alternatively, save the script and run it manually:

```bash
curl -sSL https://raw.githubusercontent.com/AtlasPilotPuppy/dedup/main/install.sh > install.sh && chmod +x install.sh

# Standard version
./install.sh

# SSH-enabled version
./install.sh --ssh
```

The script will:
1. Detect your operating system and architecture
2. Download the appropriate binary from the latest GitHub release
3. Install it to `/usr/local/bin` or `~/.local/bin` if no sudo access is available
4. Make the binary executable

### From Cargo

```bash
cargo install dedup
```

### From Source

```bash
# Clone the repository
git clone https://github.com/AtlasPilotPuppy/dedup
cd dedup

# Build in release mode
cargo build --release

# The binary will be available at target/release/dedup
```

### Windows Limitations

When using `dedups` on Windows, please note the following limitations:

1. **Path Length**: Windows has a default path length limit of 260 characters. While `dedups` can handle longer paths, you may need to enable long path support in Windows:
   - Run `git config --system core.longpaths true` if using Git
   - Enable long paths in Windows registry or group policy
   - Use the `\\?\` prefix for paths longer than 260 characters

2. **File Permissions**: Windows file permissions are more restrictive than Unix-like systems:
   - Some files may be locked by other processes
   - System files and protected directories may be inaccessible
   - Consider running as administrator for full access

3. **Media Processing**: Media deduplication on Windows requires:
   - FFmpeg installed and available in PATH
   - ImageMagick installed and available in PATH
   - Additional dependencies for video processing

4. **Performance**: Windows performance may be slightly lower than on Unix-like systems due to:
   - Different file system characteristics
   - Additional security checks
   - Path normalization overhead

5. **Configuration**: The configuration file location is different:
   - Windows: `C:\Users\<username>\.deduprc`
   - Consider using forward slashes in paths even on Windows

## Command-Line Usage

### Basic Usage

```bash
# Find duplicates in the current directory using the TUI
dedups -i

# Find duplicates in a specific directory
dedups /path/to/directory

# Find and delete duplicates (non-interactive)
dedups /path/to/directory --delete --mode newest_modified

# Use a custom config file
dedups /path/to/directory --config-file /path/to/my-config.toml
```

### Multi-Directory Operations

```bash
# Copy missing files from source to target directory
dedups /source/directory /target/directory

# Explicitly specify a target directory (can be useful with multiple source directories)
dedups /source/dir1 /source/dir2 --target /target/directory

# Deduplicate between directories and copy missing files
dedups /source/directory /target/directory --deduplicate

# Find duplicates in both source and target (without copying)
# and save the results to a file
dedups /source/directory /target/directory --deduplicate -o duplicates.json

# Copy missing files from multiple source directories to a target
dedups /source/dir1 /source/dir2 /source/dir3 /target/directory

# First deduplicate the target, then copy unique files from source
# (run as separate commands)
dedups /target/directory --delete --mode newest_modified
dedups /source/directory /target/directory
```

## Media Deduplication

The media deduplication feature can detect similar images, videos, and audio files even when they have different formats, resolutions, or quality levels.

### Supported Media Types

- **Images**: Detects similar images using perceptual hashing
- **Videos**: Extracts keyframes to identify similar video content
- **Audio**: Creates audio fingerprints to match similar audio content

### How It Works

- **Images**: Uses perceptual hashing (pHash) to create a "fingerprint" of the visual content
- **Videos**: Extracts keyframes and generates visual fingerprints
- **Audio**: Generates acoustic fingerprints that can identify similar audio content

### Media Deduplication Options

```bash
# Enable media deduplication mode
dedups /path/to/media --media-mode

# Set resolution preference (highest, lowest, or custom resolution)
dedups /path/to/media --media-mode --media-resolution highest
dedups /path/to/media --media-mode --media-resolution lowest
dedups /path/to/media --media-mode --media-resolution 1280x720

# Set format preferences (comma-separated, highest priority first)
dedups /path/to/media --media-mode --media-formats raw,png,jpg

# Adjust similarity threshold (0-100, default: 90)
dedups /path/to/media --media-mode --media-similarity 85
```

### Recommended Settings for Different Use Cases

- **Professional Photography**:
  ```bash
  dedups /path/to/photos --media-mode --media-resolution highest --media-formats raw,tiff,png,jpg
  ```

- **Web/Mobile Optimization**:
  ```bash
  dedups /path/to/images --media-mode --media-resolution 1920x1080 --media-formats webp,jpg,png
  ```

- **Audio Collection**:
  ```bash
  dedups /path/to/audio --media-mode --media-formats flac,mp3,ogg
  ```

### Sample Media Script

A sample script is included to demonstrate the media deduplication features. The script downloads small media files and creates variations with different formats, resolutions, and quality levels.

```bash
# Make the script executable
chmod +x sample_media.sh

# Run the script to create sample media files
./sample_media.sh

# Test media deduplication on the sample files (interactive mode)
dedups -i demo --media-mode

# For CLI mode with specific options
dedups --dry-run demo --media-mode --media-resolution highest --media-formats png,jpg,mp4
```

The script creates the following directory structure:

```
demo/
├── original             # Original media files
├── similar_quality      # Same media with different quality levels
├── different_formats    # Same media in different file formats
└── resized              # Same media with different resolutions
```

Dependencies for the sample script:
- curl: For downloading files
- ffmpeg: For video and audio conversions
- ImageMagick: For image conversions

### Common Workflows

#### Single Directory Cleanup

```bash
# Find and list duplicates only
dedups /path/to/photos

# Find and immediately delete duplicates, keeping newest files
dedups /path/to/photos --delete --mode newest_modified

# Move duplicates to a separate folder instead of deleting
dedups /path/to/photos --move-to /path/to/duplicates --mode shortest_path

# Export a report of duplicates for review
dedups /path/to/photos -o duplicates.json

# Use file caching for faster repeated scans
dedups /path/to/photos --cache-location ~/.dedup_cache --fast-mode
```

#### Synchronizing Directories

```bash
# Scenario 1: Safely copy missing files from source to target
dedups /source/photos /target/backup

# Scenario 2: Full synchronization with deduplication
# Step 1: Clean duplicates in the target directory
dedups /target/backup --delete --mode newest_modified
# Step 2: Clean duplicates in the source directory
dedups /source/photos --delete --mode newest_modified
# Step 3: Copy missing files from source to target
dedups /source/photos /target/backup

# Scenario 3: One-step operation to deduplicate between directories
dedups /source/photos /target/backup --deduplicate

# Scenario 4: Multiple source directories to one target
dedups /photos/2020 /photos/2021 /photos/2022 /backup/all_photos
```

### Available Options

```
USAGE:
    dedups [OPTIONS] [directory]

ARGS:
    <directory>    The directory to scan for duplicate files [default: .]

OPTIONS:
    -d, --delete                 Delete duplicate files automatically based on selection strategy
    -M, --move-to <move-to>      Move duplicate files to a specified directory
    -l, --log                    Enable logging to a file (default: dedups.log)
        --log-file <PATH>        Specify a custom log file path
    -o, --output <o>             Output duplicate sets to a file (e.g., duplicates.json)
    -f, --format <format>        Format for the output file [json|toml] [default: json]
    -a, --algorithm <algorithm>  Hashing algorithm [md5|sha1|sha256|blake3|xxhash|gxhash|fnv1a|crc32] [default: xxhash]
    -p, --parallel <parallel>    Number of parallel threads for hashing (default: auto)
        --mode <mode>            Selection strategy for delete/move [newest_modified|oldest_modified|shortest_path|longest_path] [default: newest_modified]
    -i, --interactive            Run in interactive TUI mode
    -v, --verbose...             Verbosity level (-v, -vv, -vvv)
        --include <include>...   Include specific file patterns (glob)
        --exclude <exclude>...   Exclude specific file patterns (glob)
        --filter-from <filter-from>
                                 Load filter rules from a file (one pattern per line, # for comments)
        --progress               Show progress bar for CLI scan (TUI has its own progress display)
        --sort-by <sort-by>      Sort files by criterion [name|size|created|modified|path] [default: modifiedat]
        --sort-order <sort-order>
                                 Sort order [asc|desc] [default: descending]
        --raw-sizes              Display file sizes in raw bytes instead of human-readable format
        --config-file <config-file>
                                 Path to a custom config file
        --dry-run                Perform a dry run without making any actual changes
        --cache-location <cache-location>
                                 Directory to store file hash cache for faster rescans
        --fast-mode              Use cached file hashes when available (requires cache-location)
        --media-mode             Enable media deduplication for similar images/videos/audio
        --media-resolution <resolution>
                                 Preferred resolution for media files [highest|lowest|WIDTHxHEIGHT] [default: highest]
        --media-formats <formats>
                                 Preferred formats for media files (comma-separated, e.g., 'raw,png,jpg')
        --media-similarity <threshold>
                                 Similarity threshold percentage for media files (0-100) [default: 90]
        --allow-remote-install    Allow installation of dedups on remote systems [default: true]
        --ssh-options <options>   SSH options to pass to the ssh command (comma-separated)
        --rsync-options <options> Rsync options to pass to the rsync command (comma-separated)
        --use-remote-dedups       Use remote dedups instance if available [default: true]
    -h, --help                   Print help information
    -V, --version                Print version information
```

### Filter File Format

When using `--filter-from`, the file should follow this format:

```
# This is a comment
+ *.jpg      # Include all jpg files
- *tmp*      # Exclude any path containing "tmp"
```

- Lines starting with `+` are include patterns
- Lines starting with `-` are exclude patterns
- Lines starting with `#` or `;` are comments

## Interactive TUI Mode

The TUI mode provides an interactive interface for exploring and managing duplicate sets.

### Navigation

![TUI Navigation Placeholder](#)

- **Arrow keys, j/k**: Move selection up/down
- **Tab**: Cycle between panels (Sets/Folders → Files → Jobs)
- **h/l or Left/Right**: Switch between sets and files
- **Ctrl+G**: Toggle focus on the log area
- **Ctrl+R**: Rescan

### File Operations

- **s**: Mark to keep the selected file and mark others in set for deletion
- **d**: Mark the selected file for deletion
- **c**: Copy the selected file (prompts for destination)
- **a**: Toggle all files in a set for keep/delete
- **i**: Ignore the selected file

### Bulk Actions

- **d/k**: When in the Sets panel, mark all files in the set for deletion or keeping
- **Ctrl+E**: Execute pending jobs (delete/move operations)
- **x/Delete/Backspace**: Remove the selected job

### Other Controls

- **q/Ctrl+C**: Quit the application
- **h**: Display help screen
- **Ctrl+S**: Open settings screen
- **Ctrl+L**: Clear the log area
- **Ctrl+D**: Toggle dry run mode (simulates operations without making actual changes)

### Settings

The Settings screen (Ctrl+S) allows you to configure:

- Selection strategy for keep/delete operations
- Hash algorithm
- Parallelism level
- Sort criteria and order
- Media deduplication options:
  - Media mode enable/disable
  - Resolution preference
  - Format preference
  - Similarity threshold

## Screenshots

### Main TUI Screen

![Main TUI Screen Placeholder](#)

### Settings Screen

![Settings Screen Placeholder](#)

### Help Screen

![Help Screen Placeholder](#)

## Performance Tips

- **Hash Algorithm**: xxHash (default) offers the best balance of speed and collision resistance
- **Parallelism**: Set to the number of physical cores for best performance
- **Large Directories**: Use filter patterns to narrow down the scan
- **Initial Scan**: The first scan may take longer, especially on network drives
- **File Cache**: For repeated scans of similar directories:
  - Enable `--cache-location` to store file hashes on disk
  - Use `--fast-mode` to skip hash calculations for unchanged files
  - This can dramatically speed up subsequent scans by 5-10x
- **Media Deduplication**:
  - Media scanning requires additional processing time, especially for videos
  - FFmpeg is required for video and audio processing
  - Ensure FFmpeg is installed if you want to deduplicate videos and audio

## Configuration File

`dedups` supports configuration through a `.deduprc` file in your home directory. This allows you to set default values that will be used when options are not explicitly specified on the command line.

### Location

The configuration file is located at:
- Linux/macOS: `~/.deduprc`
- Windows: `C:\Users\<username>\.deduprc`

You can also specify a custom configuration file using the `--config-file` option:
```bash
dedups --config-file /path/to/my-config.toml /path/to/directory
```

## Remote File Systems with SSH/Rsync

The SSH version of dedups supports working with remote file systems to:
- Find duplicates on remote systems
- Copy missing files between local and remote systems
- Delete or move remote duplicate files
- Deduplicate across local and remote file systems

### SSH Path Format

Remote paths are specified in the following format:
- `ssh:host:/path` - Basic format with hostname and path
- `ssh:user@host:/path` - With username
- `ssh:user@host:port:/path` - With username and port
- `ssh:host:/path:ssh_opts:rsync_opts` - With SSH and rsync options

Examples:
```bash
# Scan remote directory
dedups ssh:server.example.com:/home/user/photos

# Sync files from local to remote
dedups /local/photos ssh:user@server.example.com:/remote/backup

# Use custom SSH options
dedups ssh:server.example.com:/data:-i,~/.ssh/custom_key,-o,StrictHostKeyChecking=no

# Use custom rsync options
dedups /local/data ssh:server.example.com:/remote/data::--info=progress2,--no-perms

# Use SSH tunnel for reliable JSON streaming (for interactive progress monitoring)
dedups ssh:server.example.com:/home/user/photos --json --use-ssh-tunnel
```

### Requirements

- SSH access to the remote system
- SSH key authentication configured (password auth is not supported)
- rsync installed on both local and remote systems for file transfers
- Optional: dedups installed on the remote system for advanced features
- For SSH tunneling: `netcat` (nc) on the remote system for reliable JSON streaming

### Remote Dedups Detection

When connecting to a remote system, dedups automatically:
1. Checks if dedups is installed on the remote system
2. If found, uses the remote dedups for efficient scanning and deduplication
3. If not found, offers to install dedups on the remote system

You can control this behavior with:
- `--allow-remote-install=[true|false]` - Allow or prevent remote installation
- `--use-remote-dedups=[true|false]` - Enable or disable using remote dedups
- `--use-sudo` - Use sudo for installation (will prompt for password if needed)
- `--use-ssh-tunnel=[true|false]` - Enable or disable SSH tunneling for JSON streaming (default: true)

The installation location depends on sudo access:
- With sudo: `/usr/local/bin` (system-wide installation)
- Without sudo: `~/.local/bin` (user-specific installation)

### SSH Tunneling for JSON Output

When using remote dedups with JSON output (which is used for interactive features and progress monitoring), 
dedups uses an SSH tunnel to ensure reliable JSON streaming between the local and remote instances.

This feature:
- Creates a secure tunnel for JSON data transmission
- Uses a dedicated API-style communication channel on port 29875 (configurable)
- Separates protocol data from logs and other output
- Provides real-time progress updates from remote operations
- Uses Protocol Buffers for efficient data serialization
- Falls back to standard SSH if tunneling is unavailable

You can control this behavior with:
- `--use-ssh-tunnel` - Enable SSH tunneling (default)
- `--no-use-ssh-tunnel` - Disable SSH tunneling and use standard SSH connection
- `--tunnel-api-mode` - Use the improved API-style communication (default)
- `--no-tunnel-api-mode` - Use basic tunneling without the enhanced API separation
- `--port <number>` - Specify a custom port for the tunnel (default: 29875)

Requirements for optimal tunneling:
- SSH port forwarding permissions
- Local and remote ports available (default 29875 or auto-selected)

### Remote File Operations

All standard dedups operations work with remote paths:

```bash
# Find duplicates on a remote system
dedups ssh:server.example.com:/home/user/photos -o duplicates.json

# Find and delete duplicates on a remote system
dedups ssh:server.example.com:/home/user/photos --delete --mode newest_modified

# Copy missing files from local to remote
dedups /local/photos ssh:server.example.com:/remote/backup

# Copy missing files from remote to local
dedups ssh:server.example.com:/remote/photos /local/backup

# Move duplicate files on a remote system to a different remote directory
dedups ssh:server.example.com:/photos --move-to ssh:server.example.com:/duplicates

# Deduplicate between local and remote directories
dedups /local/photos ssh:server.example.com:/remote/photos --deduplicate
```

### Additional SSH Examples

Here are more examples showing how to use dedups with SSH/remote filesystems:

#### Delete Duplicates on Remote Host with Dry Run

Delete duplicate files on a remote host keeping the newest copies (safely test with dry run):

```bash
dedups ssh:user@example.com:/remote/photos --delete --mode newest_modified --dry-run
```

(Remove `--dry-run` to actually delete files once you're confident with the selection)

#### Move Remote Duplicates to Local Archive

Move duplicates from a remote directory to a local archive:

```bash
dedups ssh:user@example.com:/remote/photos --move-to /local/archive/duplicates
```

#### Cross-Host Deduplication

Find and handle duplicates between two remote hosts:

```bash
dedups ssh:user@server1.com:/data ssh:user@server2.com:/backup --deduplicate
```

#### Media File Deduplication on Remote Host

Find similar media files (not just exact duplicates) on a remote host:

```bash
dedups ssh:user@example.com:/photos --media-mode --media-similarity 80
```

#### Complex Example with Multiple Options

```bash
dedups /local/photos ssh:user@example.com:/remote/photos:-i,~/.ssh/custom_key:--info=progress2 \
  --deduplicate --delete --mode newest_modified --media-mode --media-similarity 85 \
  --output duplicates.json --dry-run
```

### Performance Considerations

- Using a remote dedups installation is significantly faster for large directories
- Without remote dedups, operations will be limited to basic file manipulation
- Media deduplication is not available in fallback mode (without remote dedups)
- Consider using `--algorithm` with faster hash options like xxhash for remote operations

### Security Considerations

- SSH connections use your standard SSH configuration and keys
- dedups respects your SSH configuration including key files, known hosts, etc.
- The `--allow-remote-install` option controls whether dedups can be installed remotely
- Remote installation requires write access to either `/usr/local/bin` or `~/.local/bin`

### Configuration

You can set default SSH and rsync options in your `.deduprc` file:

```toml
[ssh]
allow_remote_install = true
use_remote_dedups = true
ssh_options = ["-o", "StrictHostKeyChecking=no"]
rsync_options = ["--info=progress2"]
```

### Fallback Mode

If dedups is not installed on the remote system and cannot be installed:
- Basic file listing and manipulation will be used
- Limited hashing functionality is available
- Media deduplication is not available
- All operations will be significantly slower for large directories

## Protocol Improvements with Protobuf and ZSTD Compression

When using SSH/remote features, `dedups` now supports an improved communication protocol using Protocol Buffers (Protobuf) with optional ZSTD compression. This significantly improves performance and reduces bandwidth usage for remote operations.

### Benefits

- **Faster Communication**: Protocol Buffers offer more efficient serialization compared to JSON
- **Reduced Bandwidth**: Smaller message size for network transfers
- **Enhanced Compression**: ZSTD compression provides high compression ratios with minimal CPU usage
- **Better Performance**: Especially noticeable with large directories or high-latency connections

### How to Enable

Protocol Buffers and compression are enabled by default when using SSH features with the `proto` feature enabled, but you can control them explicitly with these options:

```bash
# Specify protocol and compression options
dedups ssh:server.example.com:/home/user/photos --use-protobuf --use-compression

# Disable Protocol Buffers (fall back to JSON)
dedups ssh:server.example.com:/home/user/photos --no-use-protobuf

# Use Protocol Buffers without compression
dedups ssh:server.example.com:/home/user/photos --use-protobuf --no-use-compression

# Adjust compression level (1-22, higher = more compression but slower)
dedups ssh:server.example.com:/home/user/photos --compression-level 9
```

### Configuration

You can set default Protocol Buffers and compression options in your `.deduprc` file:

```toml
[protocol]
use_protobuf = true
use_compression = true
compression_level = 3  # Default is 3, range is 1-22
```

### Performance Impact

- **Small Directories**: For small directories, the overhead of Protobuf might outweigh benefits
- **Large Directories**: For large directories (1000+ files), expect 2-5x faster network communication
- **Many Small Files**: When dealing with many small files, compression is particularly effective
- **High-Latency Networks**: Over VPNs or high-latency networks, the benefits increase significantly
- **Media Operations**: When using `--media-mode` remotely, expect much better performance due to reduced data transfer

### Protocol Compatibility

`dedups` automatically negotiates the protocol based on:

1. The features with which the client and server were compiled
2. The settings passed via command line or config file
3. Client/server capabilities detection

When communicating with older versions of remote `dedups`, the system will automatically fall back to JSON.

# SSH API Communication

When working with remote SSH paths, there are two modes of communication:

1. **Standard mode** (stdout parsing): Used when tunnel mode is explicitly disabled.

2. **Tunnel API mode** (DEFAULT): Creates an SSH tunnel and communicates with a dedicated API server on the remote host for more reliable operation.

## Using Tunnel API Mode

Tunnel API mode is the default for SSH communication. The system automatically:
- Establishes an SSH tunnel
- Starts a dedups server on the remote host
- Communicates using Protocol Buffers (when available)
- Applies compression for better performance
- Terminates the server when the client disconnects

For optimal performance, compile with both SSH and protocol features:

```bash
cargo build --release --features ssh,proto
```

With this build, all optimal settings are enabled by default:
```bash
dedups ssh:host:/path
```

You can be explicit about using these features:
```bash
dedups ssh:host:/path --use-ssh-tunnel --tunnel-api-mode --use-protobuf --use-compression
```

Or disable the tunnel mode (not recommended):
```bash
dedups ssh:host:/path --no-use-ssh-tunnel
```

## Run the Example

To test this functionality, run the included example script:

```bash
bash scripts/ssh_api_example.sh
```

This demonstrates both modes of operation and explains the advantages of tunnel API mode.

## Troubleshooting SSH Connections

If you encounter SSH connection issues:

1. Make sure the host is configured in your `~/.ssh/config`
2. Verify you have SSH key access to the host
3. Ensure the host is reachable
4. Check that the remote path exists

# Local API Server Mode

You can also run dedups in server mode locally for direct API communication:

```bash
# Start a dedups server on port 29876
dedups --server-mode --port 29876 /path/to/directory

# Connect to the server (in another terminal)
dedups /path/to/directory --port 29876 --json
```

To test this functionality and see protocol communication in action:

```bash
# Run the local API test script
./scripts/local_api_test.sh

# For advanced protocol testing
./scripts/local_api_test.sh --advanced
```

This demonstrates the same API protocol that is used automatically when working with SSH paths. For detailed protocol documentation, see [docs/api-protocol.md](docs/api-protocol.md).