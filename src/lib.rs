// src/lib.rs

// Re-export modules and items that integration tests (and potentially other crates) might need.

// If file_utils is a module in your src directory (e.g., src/file_utils.rs)
pub mod file_utils;

// If tui_app is a module (e.g., src/tui_app.rs or src/tui_app/mod.rs)
pub mod tui_app;

// Add the new config module
pub mod config;

// Add the file cache module
pub mod file_cache;

// Add the media deduplication module
pub mod media_dedup;

// Add audio fingerprinting module
pub mod audio_fingerprint;

// Add video fingerprinting module
pub mod video_fingerprint;

// To make Cli accessible, you'll need to move its definition from main.rs to lib.rs
// or re-export it from main.rs if main.rs uses this lib.rs as a library.
// For a typical binary project that also wants to expose a library for testing/other uses:
// Option 1: Move Cli to lib.rs
// Option 2: Keep Cli in main.rs but ensure main.rs uses `dedup::Cli` after this lib.rs is established.

// For now, let's assume you will move or already have Cli definition in a way it can be exported.
// If Cli is in main.rs, and main.rs is the binary entry point, you can't directly import from main.rs into lib.rs.
// The common pattern is to define shared structs like Cli in lib.rs and then main.rs uses them.

// Let's assume Cli will be defined here or re-exported from a module within the library.
// Placeholder for Cli - you'll need to ensure its actual definition is accessible here.
// If your Cli struct is still in main.rs, you should move it to this lib.rs file.
// For example:

use clap::Parser;
use std::path::PathBuf;
use std::str::FromStr;
// Ensure these are correctly pathed if they are part of file_utils module
use crate::config::DedupConfig;
use crate::file_utils::{SortCriterion, SortOrder};
use crate::media_dedup::MediaDedupOptions;

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// The directories to scan for duplicate or missing files.
    /// When multiple directories are specified, the last one is treated as the target
    /// for copying missing files, unless --target is specified.
    #[clap(required_unless_present = "interactive")]
    pub directories: Vec<PathBuf>,

    /// Specifies the target directory for copying missing files or deduplication.
    /// Overrides the default behavior of using the last specified directory as target.
    #[clap(long)]
    pub target: Option<PathBuf>,

    /// Whether to deduplicate between source and target directories
    /// instead of just copying missing files.
    #[clap(long, help = "Deduplicate between source and target directories")]
    pub deduplicate: bool,

    /// Automatically delete duplicate files.
    #[clap(
        short,
        long,
        help = "Delete duplicate files automatically based on selection strategy"
    )]
    pub delete: bool,

    /// Move duplicate files to the specified folder.
    #[clap(
        short = 'M',
        long,
        help = "Move duplicate files to a specified directory"
    )]
    pub move_to: Option<PathBuf>,

    /// Write actions and errors to a log file.
    #[clap(short, long, help = "Enable logging to a file (default: dedups.log)")]
    pub log: bool,

    /// Specify a custom log file path.
    #[clap(long, value_name = "PATH", help = "Specify a custom log file path")]
    pub log_file: Option<PathBuf>,

    /// Write a file containing duplicate information.
    #[clap(
        short,
        long,
        help = "Output duplicate sets to a file (e.g., duplicates.json)"
    )]
    pub output: Option<PathBuf>,

    /// Output format for the duplicates file.
    #[clap(short, long, value_parser = clap::builder::PossibleValuesParser::new(["json", "toml"]), default_value = "json", help = "Format for the output file [json|toml]")]
    pub format: String,

    /// Hashing algorithm to use for comparing files.
    #[clap(short, long, value_parser = clap::builder::PossibleValuesParser::new(["md5", "sha1", "sha256", "blake3", "xxhash", "gxhash", "fnv1a", "crc32"]), default_value = "xxhash", help = "Hashing algorithm [md5|sha1|sha256|blake3|xxhash|gxhash|fnv1a|crc32]")]
    pub algorithm: String,

    /// Number of parallel threads to use for hashing. Defaults to auto-detected number of cores.
    #[clap(
        short,
        long,
        help = "Number of parallel threads for hashing (default: auto)"
    )]
    pub parallel: Option<usize>,

    /// Mode for selecting which file to keep/delete in non-interactive mode.
    #[clap(
        long,
        default_value = "newest_modified",
        help = "Selection strategy for delete/move [newest_modified|oldest_modified|shortest_path|longest_path]"
    )]
    pub mode: String,

    /// Fire up interactive TUI mode.
    #[clap(short, long, help = "Run in interactive TUI mode")]
    pub interactive: bool,

    /// Verbosity level.
    #[clap(short, long, action = clap::ArgAction::Count, help = "Verbosity level (-v, -vv, -vvv)")]
    pub verbose: u8,

    /// Include files matching the given glob pattern. Can be specified multiple times.
    #[clap(long, help = "Include specific file patterns (glob)")]
    pub include: Vec<String>,

    /// Exclude files matching the given glob pattern. Can be specified multiple times.
    #[clap(long, help = "Exclude specific file patterns (glob)")]
    pub exclude: Vec<String>,

    /// Read filter rules from a file (similar to rclone filter files).
    #[clap(
        long,
        help = "Load filter rules from a file (one pattern per line, # for comments)"
    )]
    pub filter_from: Option<PathBuf>,

    /// Show progress information during scanning/hashing.
    #[clap(
        long,
        help = "Show progress bar for CLI scan (TUI has its own progress display)"
    )]
    pub progress: bool,

    #[clap(
        long,
        help = "Show progress during TUI scan (enabled by default for TUI mode)"
    )]
    pub progress_tui: bool,

    #[clap(long, value_parser = SortCriterion::from_str, default_value_t = SortCriterion::ModifiedAt, help = "Sort files by criterion [name|size|created|modified|path]")]
    pub sort_by: SortCriterion,

    #[clap(long, value_parser = SortOrder::from_str, default_value_t = SortOrder::Descending, help = "Sort order [asc|desc]")]
    pub sort_order: SortOrder,

    /// Display file sizes in raw bytes instead of human-readable format.
    #[clap(
        long,
        help = "Display file sizes in raw bytes instead of human-readable format"
    )]
    pub raw_sizes: bool,

    /// Path to a custom config file. If provided, overrides the default ~/.deduprc file.
    #[clap(long, help = "Path to a custom config file (overrides the default ~/.deduprc for dedups)")]
    pub config_file: Option<PathBuf>,

    /// Run in dry run mode - simulate actions without making actual changes.
    #[clap(long, help = "Perform a dry run without making any actual changes")]
    pub dry_run: bool,

    /// Directory to store hash cache for faster scanning of previously scanned files
    #[clap(long, help = "Directory to store file hash cache for faster rescans")]
    pub cache_location: Option<PathBuf>,

    /// Use cached hashes for files that haven't changed since last scan
    #[clap(
        long,
        help = "Use cached file hashes when available (requires cache-location)"
    )]
    pub fast_mode: bool,

    /// Enable media deduplication (images, videos, audio)
    #[clap(
        long,
        help = "Enable media deduplication for similar images/videos/audio"
    )]
    pub media_mode: bool,

    /// Resolution preference for media deduplication
    #[clap(long, default_value = "highest", value_parser = ["highest", "lowest"], help = "Preferred resolution for media files [highest|lowest|WIDTHxHEIGHT]")]
    pub media_resolution: String,

    /// Format preference for media files (comma-separated, highest priority first)
    #[clap(
        long,
        value_delimiter = ',',
        help = "Preferred formats for media files (comma-separated, e.g., 'raw,png,jpg')"
    )]
    pub media_formats: Vec<String>,

    /// Similarity threshold for media deduplication (0-100)
    #[clap(
        long,
        default_value = "90",
        help = "Similarity threshold percentage for media files (0-100)"
    )]
    pub media_similarity: u32,

    /// Media deduplication options (will be populated from above arguments)
    #[clap(skip)]
    pub media_dedup_options: MediaDedupOptions,
}

impl Cli {
    /// Apply configuration values from .deduprc to CLI arguments
    pub fn with_config() -> anyhow::Result<Self> {
        // Parse CLI arguments first
        let mut cli = Self::parse();

        // Initialize media_dedup_options with defaults
        cli.media_dedup_options = MediaDedupOptions::default();

        // Load configuration from specified file or default location
        let config = if let Some(config_path) = &cli.config_file {
            DedupConfig::load_from_path(config_path)?
        } else {
            DedupConfig::load()?
        };

        // Apply config values for any unspecified CLI arguments
        cli.apply_config(config);

        // Apply media deduplication options based on CLI arguments
        if cli.media_mode {
            // If media mode is enabled via CLI, update options accordingly
            crate::media_dedup::add_media_options_to_cli(
                &mut cli.media_dedup_options,
                cli.media_mode,
                &cli.media_resolution,
                &cli.media_formats,
                cli.media_similarity,
            );
        }

        // Create default config file if it doesn't exist
        // Only do this if we're using the default config path
        if cli.config_file.is_none() {
            let _ = DedupConfig::create_default_if_not_exists();
        }

        Ok(cli)
    }

    /// Apply config values to CLI arguments that weren't explicitly provided
    fn apply_config(&mut self, config: DedupConfig) {
        // Only apply config values for arguments that weren't specified on the command line

        if self.algorithm.is_empty() {
            self.algorithm = config.algorithm;
        }

        if self.parallel.is_none() {
            self.parallel = config.parallel;
        }

        if self.mode.is_empty() {
            self.mode = config.mode;
        }

        if self.format.is_empty() {
            self.format = config.format;
        }

        if !self.progress && config.progress {
            self.progress = config.progress;
        }

        // Only apply include/exclude patterns if none were specified on the command line
        if self.include.is_empty() && !config.include.is_empty() {
            self.include = config.include;
        }

        if self.exclude.is_empty() && !config.exclude.is_empty() {
            self.exclude = config.exclude;
        }

        // Apply sort_by and sort_order only if they match their default values
        // This requires special handling since they're not String types
        if self.sort_by == SortCriterion::ModifiedAt && !config.sort_by.is_empty() {
            if let Ok(sort_by) = SortCriterion::from_str(&config.sort_by) {
                self.sort_by = sort_by;
            }
        }

        if self.sort_order == SortOrder::Descending && !config.sort_order.is_empty() {
            if let Ok(sort_order) = SortOrder::from_str(&config.sort_order) {
                self.sort_order = sort_order;
            }
        }

        // Apply cache options from config if not specified on command line
        if self.cache_location.is_none() {
            self.cache_location = config.cache_location;
        }

        // Only enable fast mode if either specified on command line or in config AND cache location is available
        if !self.fast_mode && config.fast_mode {
            self.fast_mode = config.fast_mode;
        }

        // If fast mode is enabled but no cache location is specified, disable fast mode and warn
        if self.fast_mode && self.cache_location.is_none() {
            log::warn!(
                "Fast mode enabled but no cache location specified. Fast mode will be disabled."
            );
            self.fast_mode = false;
        }

        // Apply media deduplication options
        // CLI explicit flags take precedence over config file
        if !self.media_mode && config.media_dedup.enabled {
            // Apply from config if CLI didn't explicitly enable
            self.media_mode = config.media_dedup.enabled;
            self.media_dedup_options = config.media_dedup;
        }

        // Ensure we always have defaults for required fields that might be empty
        if self.algorithm.is_empty() {
            self.algorithm = "xxhash".to_string();
        }

        if self.format.is_empty() {
            self.format = "json".to_string();
        }

        if self.mode.is_empty() {
            self.mode = "newest_modified".to_string();
        }
    }
}

// If your Cli struct is already in main.rs and you want to keep it there for now (less ideal for testing library parts),
// you might need to adjust your integration tests to not depend on Cli directly if it's not easily importable.
// However, the standard way is to define such core structs in lib.rs.

// For the integration tests to compile with `use dedup::Cli;`
// You need to define or re-export `Cli` from your library crate (src/lib.rs)
// If Cli is in main.rs, consider moving it to lib.rs or a module within lib.rs.
// If you cannot move it now, the tests might need to construct a Cli-like struct or
// the tests that use Cli might need to be adjusted or temporarily disabled.

// Assuming you will make Cli available through the library crate:
// Remove the problematic 'pub use crate::main_cli_struct::Cli;' line
// The Cli struct is now defined above directly in this file.

// If Cli is in main.rs, and main.rs becomes a binary that uses this library,
// then main.rs would use `use dedup::Cli;` (if Cli is made public in lib.rs).

// Simplest path for now: Define Cli in a new module within the library, e.g. `