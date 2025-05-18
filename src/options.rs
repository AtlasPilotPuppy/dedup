use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::str::FromStr;

use crate::app_mode::AppMode;
use crate::config::DedupConfig;
use crate::file_utils::{SortCriterion, SortOrder};
use crate::media_dedup::MediaDedupOptions;

/// Centralized options structure combining CLI arguments and configuration
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct Options {
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

    /// Enable Copy Missing mode - focus on copying files not in destination
    #[clap(long, help = "Enable Copy Missing mode instead of Deduplication mode")]
    pub copy_missing: bool,

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
    #[clap(
        long,
        help = "Path to a custom config file (overrides the default ~/.deduprc for dedups)"
    )]
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

    /// Application mode - determined automatically based on arguments
    #[clap(skip)]
    pub app_mode: AppMode,
}

impl Options {
    /// Create new options instance from CLI args and config file
    pub fn new() -> Result<Self> {
        // Parse CLI arguments first
        let mut options = Self::parse();

        // Initialize media_dedup_options with defaults
        options.media_dedup_options = MediaDedupOptions::default();

        // Determine app_mode based on arguments
        options.app_mode = if options.copy_missing {
            AppMode::CopyMissing
        } else {
            AppMode::Deduplication
        };

        // Load configuration from specified file or default location
        let config = if let Some(config_path) = &options.config_file {
            DedupConfig::load_from_path(config_path)?
        } else {
            DedupConfig::load()?
        };

        // Apply config values for any unspecified CLI arguments
        options.apply_config(config);

        // Apply media deduplication options based on CLI arguments
        if options.media_mode {
            // If media mode is enabled via CLI, update options accordingly
            crate::media_dedup::add_media_options_to_cli(
                &mut options.media_dedup_options,
                options.media_mode,
                &options.media_resolution,
                &options.media_formats,
                options.media_similarity,
            );
        }

        // Create default config file if it doesn't exist
        // Only do this if we're using the default config path
        if options.config_file.is_none() {
            let _ = DedupConfig::create_default_if_not_exists();
        }

        Ok(options)
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

    /// Convert from Cli to Options - for backward compatibility during transition
    pub fn from_cli(cli: &crate::Cli) -> Self {
        let app_mode = if cli.copy_missing {
            AppMode::CopyMissing
        } else {
            AppMode::Deduplication
        };

        Self {
            directories: cli.directories.clone(),
            target: cli.target.clone(),
            deduplicate: cli.deduplicate,
            copy_missing: cli.copy_missing,
            delete: cli.delete,
            move_to: cli.move_to.clone(),
            log: cli.log,
            log_file: cli.log_file.clone(),
            output: cli.output.clone(),
            format: cli.format.clone(),
            algorithm: cli.algorithm.clone(),
            parallel: cli.parallel,
            mode: cli.mode.clone(),
            interactive: cli.interactive,
            verbose: cli.verbose,
            include: cli.include.clone(),
            exclude: cli.exclude.clone(),
            filter_from: cli.filter_from.clone(),
            progress: cli.progress,
            progress_tui: cli.progress_tui,
            sort_by: cli.sort_by,
            sort_order: cli.sort_order,
            raw_sizes: cli.raw_sizes,
            config_file: cli.config_file.clone(),
            dry_run: cli.dry_run,
            cache_location: cli.cache_location.clone(),
            fast_mode: cli.fast_mode,
            media_mode: cli.media_mode,
            media_resolution: cli.media_resolution.clone(),
            media_formats: cli.media_formats.clone(),
            media_similarity: cli.media_similarity,
            media_dedup_options: cli.media_dedup_options.clone(),
            app_mode: app_mode,
        }
    }
}
