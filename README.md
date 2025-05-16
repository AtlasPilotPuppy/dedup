# dedup

A high-performance duplicate file finder and manager written in Rust. `dedup` efficiently identifies duplicate files using parallel processing and provides both a command-line interface and an interactive Terminal User Interface (TUI) for managing the results.

## Features

- **High Performance**: Uses multi-threading with Rayon for parallel hash calculation
- **Multiple Hash Algorithms**: Choose between MD5, SHA1, SHA256, Blake3, xxHash (default), GxHash, FNV1a, or CRC32
- **Interactive TUI**: Visually inspect and manage duplicate files
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
cargo install dedup_tui
```

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/dedup_tui.git
cd dedup_tui

# Build in release mode
cargo build --release

# The binary will be available at target/release/dedup_tui
```

## Command-Line Usage

### Basic Usage

```bash
# Find duplicates in the current directory using the TUI
dedup_tui -i

# Find duplicates in a specific directory
dedup_tui /path/to/directory

# Find and delete duplicates (non-interactive)
dedup_tui /path/to/directory --delete --mode newest_modified

# Use a custom config file
dedup_tui /path/to/directory --config-file /path/to/my-config.toml
```

### Multi-Directory Operations

```bash
# Copy missing files from source to target directory
dedup_tui /source/directory /target/directory

# Explicitly specify a target directory (can be useful with multiple source directories)
dedup_tui /source/dir1 /source/dir2 --target /target/directory

# Deduplicate between directories and copy missing files
dedup_tui /source/directory /target/directory --deduplicate

# Find duplicates in both source and target (without copying)
# and save the results to a file
dedup_tui /source/directory /target/directory --deduplicate -o duplicates.json

# Copy missing files from multiple source directories to a target
dedup_tui /source/dir1 /source/dir2 /source/dir3 /target/directory

# First deduplicate the target, then copy unique files from source
# (run as separate commands)
dedup_tui /target/directory --delete --mode newest_modified
dedup_tui /source/directory /target/directory
```

### Common Workflows

#### Single Directory Cleanup

```bash
# Find and list duplicates only
dedup_tui /path/to/photos

# Find and immediately delete duplicates, keeping newest files
dedup_tui /path/to/photos --delete --mode newest_modified

# Move duplicates to a separate folder instead of deleting
dedup_tui /path/to/photos --move-to /path/to/duplicates --mode shortest_path

# Export a report of duplicates for review
dedup_tui /path/to/photos -o duplicates.json
```

#### Synchronizing Directories

```bash
# Scenario 1: Safely copy missing files from source to target
dedup_tui /source/photos /target/backup

# Scenario 2: Full synchronization with deduplication
# Step 1: Clean duplicates in the target directory
dedup_tui /target/backup --delete --mode newest_modified
# Step 2: Clean duplicates in the source directory
dedup_tui /source/photos --delete --mode newest_modified
# Step 3: Copy missing files from source to target
dedup_tui /source/photos /target/backup

# Scenario 3: One-step operation to deduplicate between directories
dedup_tui /source/photos /target/backup --deduplicate

# Scenario 4: Multiple source directories to one target
dedup_tui /photos/2020 /photos/2021 /photos/2022 /backup/all_photos
```

### Available Options

```
USAGE:
    dedup_tui [OPTIONS] [directory]

ARGS:
    <directory>    The directory to scan for duplicate files [default: .]

OPTIONS:
    -d, --delete                 Delete duplicate files automatically based on selection strategy
    -M, --move-to <move-to>      Move duplicate files to a specified directory
    -l, --log                    Log actions and errors to a file (dedup.log)
    -o, --output <o>        Output duplicate sets to a file (e.g., duplicates.json)
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

## Screenshots

### Main TUI Screen

![Main TUI Screen Placeholder](#)

### Settings Screen

![Settings Screen Placeholder](#)

### Help Screen

![Help Screen Placeholder](#)

## Performance Tips

- **Hash Algorithm**: Blake3 (default) is the fastest, followed by MD5 then SHA256
- **Parallelism**: Set to the number of physical cores for best performance
- **Large Directories**: Use filter patterns to narrow down the scan
- **Initial Scan**: The first scan may take longer, especially on network drives

## Configuration File

`dedup` supports configuration through a `.deduprc` file in your home directory. This allows you to set default values that will be used when options are not explicitly specified on the command line.

### Location

The configuration file is located at:
- Linux/macOS: `~/.deduprc`
- Windows: `C:\Users\<username>\.deduprc`

You can also specify a custom configuration file using the `--config-file` option:
```bash
dedup_tui --config-file /path/to/my-config.toml /path/to/directory
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
git clone https://github.com/yourusername/dedup_tui.git
cd dedup_tui

# Install development dependencies
cargo install cargo-watch cargo-tarpaulin

# Run tests in watch mode
cargo watch -x test

# Generate test coverage
cargo tarpaulin --out Html
```

## License

This project is licensed under the MIT License - see the LICENSE file for details. 