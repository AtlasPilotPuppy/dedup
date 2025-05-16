// src/lib.rs

// Re-export modules and items that integration tests (and potentially other crates) might need.

// If file_utils is a module in your src directory (e.g., src/file_utils.rs)
pub mod file_utils;

// If tui_app is a module (e.g., src/tui_app.rs or src/tui_app/mod.rs)
pub mod tui_app;

// To make Cli accessible, you'll need to move its definition from main.rs to lib.rs
// or re-export it from main.rs if main.rs uses this lib.rs as a library.
// For a typical binary project that also wants to expose a library for testing/other uses:
// Option 1: Move Cli to lib.rs
// Option 2: Keep Cli in main.rs but ensure main.rs uses `dedup_tui::Cli` after this lib.rs is established.

// For now, let's assume you will move or already have Cli definition in a way it can be exported.
// If Cli is in main.rs, and main.rs is the binary entry point, you can't directly import from main.rs into lib.rs.
// The common pattern is to define shared structs like Cli in lib.rs and then main.rs uses them.

// Let's assume Cli will be defined here or re-exported from a module within the library.
// Placeholder for Cli - you'll need to ensure its actual definition is accessible here.
// If your Cli struct is still in main.rs, you should move it to this lib.rs file.
// For example:

use clap::Parser;
use std::path::PathBuf;
// Ensure these are correctly pathed if they are part of file_utils module
use crate::file_utils::{SortCriterion, SortOrder};

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// The directory to scan for duplicate files.
    #[clap(required_unless_present = "interactive", default_value = ".")]
    pub directory: PathBuf,

    /// Automatically delete duplicate files.
    #[clap(short, long, help = "Delete duplicate files automatically based on selection strategy")]
    pub delete: bool,

    /// Move duplicate files to the specified folder.
    #[clap(short = 'M', long, help = "Move duplicate files to a specified directory")]
    pub move_to: Option<PathBuf>,

    /// Write a log file to the specified path.
    #[clap(short, long, help = "Log actions and errors to a file (dedup.log)")]
    pub log: bool,

    /// Write a file containing duplicate information.
    #[clap(short, long, help = "Output duplicate sets to a file (e.g., duplicates.json)")]
    pub output: Option<PathBuf>,

    /// Output format for the duplicates file.
    #[clap(short, long, value_parser = clap::builder::PossibleValuesParser::new(["json", "toml"]), default_value = "json", help = "Format for the output file [json|toml]")]
    pub format: String,

    /// Hashing algorithm to use for comparing files.
    #[clap(short, long, value_parser = clap::builder::PossibleValuesParser::new(["md5", "sha256", "blake3"]), default_value = "blake3", help = "Hashing algorithm [md5|sha256|blake3]")]
    pub algorithm: String,

    /// Number of parallel threads to use for hashing. Defaults to auto-detected number of cores.
    #[clap(short, long, help = "Number of parallel threads for hashing (default: auto)")]
    pub parallel: Option<usize>,

    /// Mode for selecting which file to keep/delete in non-interactive mode.
    #[clap(long, default_value = "newest", help = "Selection strategy for delete/move [newest|oldest|shortest|longest]")]
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
    #[clap(long, help = "Load filter rules from a file (one pattern per line, # for comments)")]
    pub filter_from: Option<PathBuf>,

    /// Show progress information during scanning/hashing.
    #[clap(long, help = "Show progress bar for CLI scan (TUI has its own progress display)")]
    pub progress: bool,

    #[clap(long, help = "Show progress during TUI scan (enabled by default for TUI mode)")]
    pub progress_tui: bool,

    #[clap(long, value_parser = SortCriterion::from_str, default_value = "modifiedat", help = "Sort files by criterion [name|size|created|modified|path]")]
    pub sort_by: SortCriterion,

    #[clap(long, value_parser = SortOrder::from_str, default_value = "descending", help = "Sort order [asc|desc]")]
    pub sort_order: SortOrder,

    /// Display file sizes in raw bytes instead of human-readable format.
    #[clap(long, help = "Display file sizes in raw bytes instead of human-readable format")]
    pub raw_sizes: bool,
}

// If your Cli struct is already in main.rs and you want to keep it there for now (less ideal for testing library parts),
// you might need to adjust your integration tests to not depend on Cli directly if it's not easily importable.
// However, the standard way is to define such core structs in lib.rs.

// For the integration tests to compile with `use dedup_tui::Cli;`
// You need to define or re-export `Cli` from your library crate (src/lib.rs)
// If Cli is in main.rs, consider moving it to lib.rs or a module within lib.rs.
// If you cannot move it now, the tests might need to construct a Cli-like struct or
// the tests that use Cli might need to be adjusted or temporarily disabled.

// Assuming you will make Cli available through the library crate:
// Remove the problematic 'pub use crate::main_cli_struct::Cli;' line
// The Cli struct is now defined above directly in this file.

// If Cli is in main.rs, and main.rs becomes a binary that uses this library,
// then main.rs would use `use dedup_tui::Cli;` (if Cli is made public in lib.rs).

// Simplest path for now: Define Cli in a new module within the library, e.g. `src/cli_definition.rs`
// then `pub mod cli_definition;` and `pub use cli_definition::Cli;` here. 