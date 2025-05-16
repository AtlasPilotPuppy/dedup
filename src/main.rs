mod file_utils;
mod tui_app;

use clap::Parser;
use std::path::PathBuf;
use simplelog::*;
use std::fs::File;
use anyhow::Result;
use humansize::{format_size, DECIMAL};

#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// The directory to scan for duplicate files.
    #[clap(required_unless_present = "interactive", default_value = ".")]
    directory: PathBuf,

    /// Automatically delete duplicate files.
    #[clap(short, long)]
    delete: bool,

    /// Move duplicate files to the specified folder.
    #[clap(short = 'M', long = "move-to")] // Changed short to 'M' to avoid conflict
    move_to: Option<PathBuf>,

    /// Write a log file to the specified path.
    #[clap(short, long)]
    log: Option<PathBuf>,

    /// Write a file containing duplicate information.
    #[clap(short, long, default_value = "duplicates.json")]
    output: PathBuf,

    /// Output format for the duplicates file.
    #[clap(short, long, value_parser = ["json", "toml"], default_value = "json")]
    format: String,

    /// Hashing algorithm to use for comparing files.
    #[clap(short, long, value_parser = ["md5", "sha256", "blake3"], default_value = "blake3")]
    algorithm: String,

    /// Number of parallel threads to use for hashing. Defaults to auto-detected number of cores.
    #[clap(short, long)]
    parallel: Option<usize>,

    /// Mode for selecting which file to keep/delete in non-interactive mode.
    #[clap(long, value_parser = ["shortest_path", "longest_path", "newest_modified", "oldest_modified"], default_value = "newest_modified")]
    mode: String,

    /// Fire up interactive TUI mode.
    #[clap(short, long)]
    interactive: bool,

    /// Verbosity level.
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Include files matching the given glob pattern. Can be specified multiple times.
    #[clap(long)]
    include: Option<Vec<String>>,

    /// Exclude files matching the given glob pattern. Can be specified multiple times.
    #[clap(long)]
    exclude: Option<Vec<String>>,

    /// Read filter rules from a file (similar to rclone filter files).
    #[clap(long)]
    filter_from: Option<PathBuf>,

    /// Show progress information during scanning/hashing.
    #[clap(long)]
    progress: bool,
}

fn setup_logger(verbosity: u8, log_file: Option<&PathBuf>) -> Result<()> {
    let log_level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();

    let term_logger = TermLogger::new(
        log_level,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    );
    loggers.push(term_logger);

    if let Some(path) = log_file {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = File::create(path)?;
        let file_logger = WriteLogger::new(log_level, Config::default(), file);
        loggers.push(file_logger);
    }

    CombinedLogger::init(loggers)?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    setup_logger(cli.verbose, cli.log.as_ref())?;

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

                    // Output to file if specified (and not using default if it's just for display)
                    // The default output file is "duplicates.json". We should write to it unless the user changes it.
                    // Or, perhaps, only write if the user *explicitly* sets -o or if it's not the default and implicit?
                    // For now, let's assume we always write to cli.output if duplicates are found.
                    if !cli.output.as_os_str().is_empty() { // Check if output path is provided
                        match file_utils::output_duplicates(&duplicate_sets, &cli.output, &cli.format) {
                            Ok(_) => {
                                log::info!("Successfully wrote duplicate list to {:?}", cli.output);
                                println!("Duplicate list saved to {:?}", cli.output);
                            }
                            Err(e) => {
                                log::error!("Failed to write duplicate list to {:?}: {}", cli.output, e);
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
                                        // Note: No dry_run flag from CLI yet, so actions are real.
                                        match file_utils::delete_files(&files_to_action, false) {
                                            Ok(count) => total_deleted += count,
                                            Err(e) => log::error!("Error during deletion batch: {}", e),
                                        }
                                    } else if let Some(ref target_move_dir) = cli.move_to {
                                        match file_utils::move_files(&files_to_action, target_move_dir, false) {
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
