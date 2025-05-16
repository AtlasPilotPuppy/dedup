mod file_utils;
mod tui_app;

use clap::Parser;
use std::path::{Path, PathBuf};
use simplelog::LevelFilter;
use anyhow::Result;
use humansize::{format_size, DECIMAL};
use env_logger;

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

    #[clap(long, value_parser = crate::file_utils::SortCriterion::from_str, default_value = "modifiedat", help = "Sort files by criterion [name|size|created|modified|path]")]
    pub sort_by: crate::file_utils::SortCriterion,

    #[clap(long, value_parser = crate::file_utils::SortOrder::from_str, default_value = "descending", help = "Sort order [asc|desc]")]
    pub sort_order: crate::file_utils::SortOrder,

    /// Display file sizes in raw bytes instead of human-readable format.
    #[clap(long, help = "Display file sizes in raw bytes instead of human-readable format")]
    pub raw_sizes: bool,
}

fn setup_logger(verbosity: u8, log_file: Option<&Path>) -> Result<()> {
    let level = match verbosity {
        0 => LevelFilter::Info,
        1 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let mut builder = env_logger::Builder::new();
    builder.filter_level(level);
    builder.format_timestamp_millis();
    builder.format_target(false);

    if let Some(log_path) = log_file {
        let file = std::fs::File::create(log_path)?;
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }

    builder.init();
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    setup_logger(cli.verbose, cli.log.then_some(&PathBuf::from("dedup.log").as_path()))?;

    log::info!("Logger initialized. Application starting.");
    log::debug!("CLI args: {:#?}", cli);

    if !cli.directory.exists() {
        log::error!("Target directory {:?} does not exist.", cli.directory);
        return Err(anyhow::anyhow!("Target directory does not exist."));
    }
    if !cli.directory.is_dir() {
        log::error!("Target path {:?} is not a directory.", cli.directory);
        return Err(anyhow::anyhow!("Target path is not a directory."));
    }

    if cli.interactive {
        log::info!("Interactive mode selected for directory: {:?}", cli.directory);
        tui_app::run_tui_app(&cli)?
    } else {
        log::info!("Non-interactive mode selected for directory: {:?}", cli.directory);
        match file_utils::find_duplicate_files(&cli) {
            Ok(duplicate_sets) => {
                if duplicate_sets.is_empty() {
                    log::info!("No duplicate files found.");
                    println!("No duplicate files found.");
                } else {
                    log::info!("Found {} sets of duplicate files.", duplicate_sets.len());
                    println!("Found {} sets of duplicate files:", duplicate_sets.len());
                    for set in &duplicate_sets {
                        println!("  Duplicates ({} files, size: {}, hash: {}...):", 
                                 set.files.len(), 
                                 format_size(set.size, DECIMAL),
                                 set.hash.chars().take(16).collect::<String>());
                        for file_info in &set.files {
                            println!("    - {}", file_info.path.display());
                        }
                    }

                    // Output to file if specified
                    if let Some(output_path) = &cli.output {
                        match file_utils::output_duplicates(&duplicate_sets, output_path, &cli.format) {
                            Ok(_) => {
                                log::info!("Successfully wrote duplicate list to {:?}", output_path);
                                println!("Duplicate list saved to {:?}", output_path);
                            }
                            Err(e) => {
                                log::error!("Failed to write duplicate list to {:?}: {}", output_path, e);
                                eprintln!("Failed to write output file: {}", e);
                            }
                        }
                    }
                    
                    // Perform actions if requested
                    if cli.delete || cli.move_to.is_some() {
                        let strategy = file_utils::SelectionStrategy::from_str(&cli.mode)?;
                        let mut total_deleted = 0;
                        let mut total_moved = 0;

                        for set in &duplicate_sets {
                            if set.files.len() < 2 { continue; }

                            match file_utils::determine_action_targets(set, strategy) {
                                Ok((kept_file, files_to_action)) => {
                                    log::info!("For duplicate set (hash: {}...), keeping file: {:?}", 
                                             set.hash.chars().take(8).collect::<String>(), 
                                             kept_file.path);
                                    println!("Keeping: {}", kept_file.path.display());

                                    if cli.delete {
                                        match file_utils::delete_files(&files_to_action, cli.progress) {
                                            Ok(count) => total_deleted += count,
                                            Err(e) => log::error!("Error during deletion batch: {}", e),
                                        }
                                    } else if let Some(ref target_move_dir) = cli.move_to {
                                        match file_utils::move_files(&files_to_action, target_move_dir, cli.progress) {
                                            Ok(count) => total_moved += count,
                                            Err(e) => log::error!("Error during move batch: {}", e),
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!("Could not determine action targets for a set: {}", e);
                                }
                            }
                        }
                        if cli.delete {
                            log::info!("Total files deleted: {}", total_deleted);
                            println!("Total files deleted: {}", total_deleted);
                        }
                        if cli.move_to.is_some() {
                            log::info!("Total files moved: {}", total_moved);
                            println!("Total files moved: {}", total_moved);
                        }
                    } else {
                        log::info!("No action flags (--delete or --move-to) specified. Listing duplicates only.");
                    }
                }
            }
            Err(e) => {
                log::error!("Error finding duplicate files: {}", e);
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}
