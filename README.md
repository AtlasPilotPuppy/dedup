# dedup

A high-performance duplicate file finder and manager written in Rust. `dedup` efficiently identifies duplicate files using parallel processing and provides both a command-line interface and an interactive Terminal User Interface (TUI) for managing the results.

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

When using `dedup` on Windows, please note the following limitations:

1. **Path Length**: Windows has a default path length limit of 260 characters. While `dedup` can handle longer paths, you may need to enable long path support in Windows:
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
dedup -i

# Find duplicates in a specific directory
dedup /path/to/directory

# Find and delete duplicates (non-interactive)
dedup /path/to/directory --delete --mode newest_modified

# Use a custom config file
dedup /path/to/directory --config-file /path/to/my-config.toml
```

### Multi-Directory Operations

```bash
# Copy missing files from source to target directory
dedup /source/directory /target/directory

# Explicitly specify a target directory (can be useful with multiple source directories)
dedup /source/dir1 /source/dir2 --target /target/directory

# Deduplicate between directories and copy missing files
dedup /source/directory /target/directory --deduplicate

# Find duplicates in both source and target (without copying)
# and save the results to a file
dedup /source/directory /target/directory --deduplicate -o duplicates.json

# Copy missing files from multiple source directories to a target
dedup /source/dir1 /source/dir2 /source/dir3 /target/directory

# First deduplicate the target, then copy unique files from source
# (run as separate commands)
dedup /target/directory --delete --mode newest_modified
dedup /source/directory /target/directory
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
dedup /path/to/media --media-mode

# Set resolution preference (highest, lowest, or custom resolution)
dedup /path/to/media --media-mode --media-resolution highest
dedup /path/to/media --media-mode --media-resolution lowest
dedup /path/to/media --media-mode --media-resolution 1280x720

# Set format preferences (comma-separated, highest priority first)
dedup /path/to/media --media-mode --media-formats raw,png,jpg

# Adjust similarity threshold (0-100, default: 90)
dedup /path/to/media --media-mode --media-similarity 85
```

### Recommended Settings for Different Use Cases

- **Professional Photography**:
  ```bash
  dedup /path/to/photos --media-mode --media-resolution highest --media-formats raw,tiff,png,jpg
  ```

- **Web/Mobile Optimization**:
  ```bash
  dedup /path/to/images --media-mode --media-resolution 1920x1080 --media-formats webp,jpg,png
  ```

- **Audio Collection**:
  ```bash
  dedup /path/to/audio --media-mode --media-formats flac,mp3,ogg
  ```

### Sample Media Script

A sample script is included to demonstrate the media deduplication features. The script downloads small media files and creates variations with different formats, resolutions, and quality levels.

```bash
# Make the script executable
chmod +x sample_media.sh

# Run the script to create sample media files
./sample_media.sh

# Test media deduplication on the sample files (interactive mode)
dedup -i demo --media-mode

# For CLI mode with specific options
dedup --dry-run demo --media-mode --media-resolution highest --media-formats png,jpg,mp4
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
dedup /path/to/photos

# Find and immediately delete duplicates, keeping newest files
dedup /path/to/photos --delete --mode newest_modified

# Move duplicates to a separate folder instead of deleting
dedup /path/to/photos --move-to /path/to/duplicates --mode shortest_path

# Export a report of duplicates for review
dedup /path/to/photos -o duplicates.json

# Use file caching for faster repeated scans
dedup /path/to/photos --cache-location ~/.dedup_cache --fast-mode
```

#### Synchronizing Directories

```bash
# Scenario 1: Safely copy missing files from source to target
dedup /source/photos /target/backup

# Scenario 2: Full synchronization with deduplication
# Step 1: Clean duplicates in the target directory
dedup /target/backup --delete --mode newest_modified
# Step 2: Clean duplicates in the source directory
dedup /source/photos --delete --mode newest_modified
# Step 3: Copy missing files from source to target
dedup /source/photos /target/backup

# Scenario 3: One-step operation to deduplicate between directories
dedup /source/photos /target/backup --deduplicate

# Scenario 4: Multiple source directories to one target
dedup /photos/2020 /photos/2021 /photos/2022 /backup/all_photos
```

### Available Options

```
USAGE:
    dedup [OPTIONS] [directory]

ARGS:
    <directory>    The directory to scan for duplicate files [default: .]

OPTIONS:
    -d, --delete                 Delete duplicate files automatically based on selection strategy
    -M, --move-to <move-to>      Move duplicate files to a specified directory
    -l, --log                    Enable logging to a file (default: dedup.log)
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

`dedup` supports configuration through a `.deduprc` file in your home directory. This allows you to set default values that will be used when options are not explicitly specified on the command line.

### Location

The configuration file is located at:
- Linux/macOS: `~/.deduprc`
- Windows: `C:\Users\<username>\.deduprc`

You can also specify a custom configuration file using the `--config-file` option:
```bash
dedup --config-file /path/to/my-config.toml /path/to/directory
```

### Format

The configuration file uses TOML format:

```toml
# Default hashing algorithm
algorithm = "blake3"

# Default number of parallel threads (leave empty for auto-detection)
parallel = 4

# Default selection strategy for delete/move operations
mode = "newest_modified"

# Default output format
format = "json"

# Whether to show progress by default
progress = true

# Default sorting options
sort_by = "modifiedat"
sort_order = "descending"

# Default file patterns to include
include = ["*.jpg", "*.png", "*.mp4"]

# Default file patterns to exclude
exclude = ["*tmp*", "*.log"]

# File hash cache settings
cache_location = "/path/to/cache"  # Optional: Path to store hash cache
fast_mode = false                  # Whether to use cache by default

# Media deduplication settings
[media_dedup]
enabled = true
resolution_preference = "highest"  # highest, lowest, or WIDTHxHEIGHT
similarity_threshold = 90          # 0-100, where 100 is exact match

# Format preferences (ordered by preference, highest first)
formats = [
  "raw",
  "png",
  "jpg",
  "mp4",
  "flac",
  "mp3"
]
```

### Usage

The application will automatically create a default configuration file if one doesn't exist. Options specified on the command line will always take precedence over the configuration file.

If you specify a custom configuration file with `--config-file`, the default `.deduprc` file will be ignored, and no default file will be created if it doesn't exist.

## Contributing

Contributions are welcome! Here's how you can help:

1. **Fork the repository**
2. **Create a feature branch**:
   ```bash
   git checkout -b feature/amazing-feature
   ```
3. **Make your changes**
4. **Run the tests**:
   ```bash
   cargo test
   ```
5. **Commit your changes**:
   ```bash
   git commit -m 'Add some amazing feature'
   ```
6. **Push to the branch**:
   ```bash
   git push origin feature/amazing-feature
   ```
7. **Open a Pull Request**

### Development Setup

To set up the development environment:

```bash
# Clone the repository
git clone https://github.com/yourusername/dedup.git
cd dedup

# Install development dependencies
cargo install cargo-watch cargo-tarpaulin

# Run tests in watch mode
cargo watch -x test

# Generate test coverage
cargo tarpaulin --out Html
```

## License

This project is licensed under the MIT License - see the LICENSE file for details. 