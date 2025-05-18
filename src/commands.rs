use anyhow::Result;
use humansize::{format_size, DECIMAL};
use simplelog::LevelFilter;
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc;

use crate::file_utils::{DuplicateSet, SelectionStrategy};
use crate::options::Options;
use crate::tui_app;

/// Set up the logger based on verbosity level and log file
pub fn setup_logger(verbosity: u8, log_file: Option<&Path>) -> Result<()> {
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
        // Create the file explicitly to handle errors better
        let file = match std::fs::File::create(log_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Warning: Could not create log file {:?}: {}", log_path, e);
                // If we're in TUI mode, we shouldn't fall back to stderr as it would break the UI
                return Ok(()); // Return without initializing logger if we can't create the file
            }
        };
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }

    // Initialize the logger and handle any errors
    match builder.try_init() {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Warning: Could not initialize logger: {}", e);
            Ok(()) // Continue without logging rather than failing
        }
    }
}

/// Run the application using the provided options
pub fn run_app(options: &Options) -> Result<()> {
    // Configure logging based on mode
    if options.interactive {
        // Interactive mode logging is handled inside run_interactive_mode
        // No need to set up logging here
    } else if options.log || options.log_file.is_some() {
        // User enabled logging
        let log_path = if let Some(path) = &options.log_file {
            path.as_path()
        } else {
            Path::new("dedups.log")
        };
        setup_logger(options.verbose, Some(log_path))?;
    } else if options.progress {
        // CLI progress display - use terminal logger
        simplelog::TermLogger::init(
            match options.verbose {
                0 => LevelFilter::Info,
                1 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            },
            simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )?;
    } else {
        // No special requirements - use simple logger
        simplelog::SimpleLogger::init(
            match options.verbose {
                0 => LevelFilter::Info,
                1 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            },
            simplelog::Config::default(),
        )?;
    }

    if !options.interactive {
        // Only log this outside of interactive mode to avoid console output
        log::info!("Logger initialized. Application starting.");
        log::debug!("CLI args: {:#?}", options);
    }

    // Check if directories exist
    for dir in &options.directories {
        if !dir.exists() {
            log::error!("Target directory {:?} does not exist.", dir);
            return Err(anyhow::anyhow!(
                "Target directory does not exist: {:?}",
                dir
            ));
        }
        if !dir.is_dir() {
            log::error!("Target path {:?} is not a directory.", dir);
            return Err(anyhow::anyhow!("Target path is not a directory: {:?}", dir));
        }
    }

    // Determine which mode to run based on app_mode
    match options.app_mode {
        crate::app_mode::AppMode::Deduplication => {
            // Handle deduplication mode (existing code)
            if options.interactive {
                run_interactive_mode(options)
            } else {
                // Check if we're comparing multiple directories
                let is_multi_directory = options.directories.len() > 1 || options.target.is_some();
                
                if is_multi_directory {
                    // Multiple directory mode - handling copying missing files or deduplication
                    handle_multi_directory_mode(options)
                } else {
                    // Single directory mode - find duplicates within one directory
                    handle_single_directory_mode(options)
                }
            }
        },
        crate::app_mode::AppMode::CopyMissing => {
            // Handle copy missing mode
            if options.interactive {
                run_copy_missing_interactive_mode(options)
            } else {
                handle_copy_missing_mode(options)
            }
        }
    }
}

/// Run the application in interactive TUI mode
fn run_interactive_mode(options: &Options) -> Result<()> {
    // Always set up file-based logging for TUI mode to avoid console disruption
    // If log_file is specified, use that, otherwise use a default location
    let log_path = options.log_file.as_ref().map(|p| p.as_path()).unwrap_or_else(|| Path::new("dedups.log"));
    
    // Make sure logging is set up before any log calls
    if let Err(e) = setup_logger(options.verbose, Some(log_path)) {
        eprintln!("Warning: Failed to set up logging: {}", e);
        // Continue anyway, just without logging
    }
    
    log::info!(
        "Interactive mode selected for directories: {:?}",
        options.directories
    );
    
    // Create a copy of options with progress_tui set to true to ensure proper message passing
    let mut tui_options = options.clone();
    tui_options.progress_tui = true;
    
    // Call the unified TUI function with copy_missing set to false for regular deduplication mode
    tui_app::run_tui_app_for_mode(&tui_options, false)
}

/// Handle multiple directory mode - comparing directories and copying/deduplicating
fn handle_multi_directory_mode(options: &Options) -> Result<()> {
    log::info!("Multi-directory mode: Comparing directories");
    println!("Comparing directories for missing files or duplicates...");

    // If in dry run mode, show banner
    if options.dry_run {
        log::info!("Running in DRY RUN mode - no files will be modified");
        println!("\n===== DRY RUN MODE - NO FILES WILL BE MODIFIED =====\n");
    }

    let target_dir = crate::file_utils::determine_target_directory(options)?;
    let source_dirs = crate::file_utils::get_source_directories(options, &target_dir);

    println!("Source directories: {:?}", source_dirs);
    println!("Target directory: {:?}", target_dir);

    let comparison_result = crate::file_utils::compare_directories(options)?;

    // Handle missing files
    if !comparison_result.missing_in_target.is_empty() {
        println!(
            "Found {} files that exist in source but not in target.",
            comparison_result.missing_in_target.len()
        );

        if options.deduplicate {
            println!("Deduplication mode enabled. Missing files will be considered separately from duplicates.");
        }

        if options.delete {
            println!("Warning: Delete flag is ignored for missing files. Use --deduplicate to handle duplicates.");
        } else if options.move_to.is_some() {
            println!("Warning: Move flag is ignored for missing files. They will be copied to the target directory.");
        }

        // Copy missing files to target directory
        match crate::file_utils::copy_missing_files(
            &comparison_result.missing_in_target,
            &target_dir,
            options.dry_run,
        ) {
            Ok((count, logs)) => {
                // Display all log messages
                for log_msg in logs {
                    // Only log to file what hasn't already been logged in the function
                    if !log_msg.starts_with("[DRY RUN]") {
                        log::info!("{}", log_msg);
                    }
                    println!("{}", log_msg);
                }

                // Adjust the summary message based on dry run mode
                let action_prefix = if options.dry_run {
                    "[DRY RUN] Would have copied"
                } else {
                    "Successfully copied"
                };
                println!("\n{} {} files to target directory.", action_prefix, count);
            }
            Err(e) => {
                log::error!("Failed to copy files: {}", e);
                eprintln!("Error copying files: {}", e);
            }
        }
    } else {
        println!("No missing files found in target directory.");
    }

    // Handle duplicates if deduplication is enabled
    if options.deduplicate && !comparison_result.duplicates.is_empty() {
        println!(
            "Found {} duplicate sets across source and target directories.",
            comparison_result.duplicates.len()
        );

        // Process duplicates similar to single directory mode
        handle_duplicate_sets(options, &comparison_result.duplicates)?;
    } else if options.deduplicate {
        println!("No duplicate files found across source and target directories.");
    }

    // Add final reminder if in dry run mode
    if options.dry_run {
        println!("\nThis was a dry run. No files were actually modified.");
        println!("Run without --dry-run to perform actual operations.");
        log::info!("Dry run completed - no files were modified");
    }

    Ok(())
}

/// Handle single directory mode - find duplicates within one directory
fn handle_single_directory_mode(options: &Options) -> Result<()> {
    log::info!(
        "Non-interactive mode selected for directory: {:?}",
        options.directories[0]
    );

    // Since we're not in TUI mode, we need a channel to receive progress updates
    let (tx, _rx) = mpsc::channel();

    match crate::file_utils::find_duplicate_files_with_progress(options, tx) {
        Ok(duplicate_sets) => {
            if duplicate_sets.is_empty() {
                log::info!("No duplicate files found.");
                println!("No duplicate files found.");
            } else {
                handle_duplicate_sets(options, &duplicate_sets)?;
            }
        }
        Err(e) => {
            log::error!("Error finding duplicate files: {}", e);
            eprintln!("Error: {}", e);
        }
    }

    Ok(())
}

/// Handle duplicate sets (common code for both single and multi-directory modes)
fn handle_duplicate_sets(options: &Options, duplicate_sets: &[DuplicateSet]) -> Result<()> {
    log::info!("Found {} sets of duplicate files.", duplicate_sets.len());
    println!("Found {} sets of duplicate files:", duplicate_sets.len());

    for set in duplicate_sets {
        println!(
            "  Duplicates ({} files, size: {}, hash: {}...):",
            set.files.len(),
            format_size(set.size, DECIMAL),
            set.hash.chars().take(16).collect::<String>()
        );
        for file_info in &set.files {
            println!("    - {}", file_info.path.display());
        }
    }

    if let Some(output_path) = &options.output {
        match crate::file_utils::output_duplicates(duplicate_sets, output_path, &options.format) {
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

    if options.delete || options.move_to.is_some() {
        // Log dry run mode status at the beginning
        if options.dry_run {
            log::info!("Running in DRY RUN mode - no files will be modified");
            println!("\n===== DRY RUN MODE - NO FILES WILL BE MODIFIED =====\n");
        }

        let strategy = SelectionStrategy::from_str(&options.mode)?;
        let mut total_deleted = 0;
        let mut total_moved = 0;

        for set in duplicate_sets {
            if set.files.len() < 2 {
                continue;
            }

            match crate::file_utils::determine_action_targets(set, strategy) {
                Ok((kept_file, files_to_action)) => {
                    log::info!(
                        "For duplicate set (hash: {}...), keeping file: {:?}",
                        set.hash.chars().take(8).collect::<String>(),
                        kept_file.path
                    );
                    println!("Keeping: {}", kept_file.path.display());

                    if options.delete {
                        match crate::file_utils::delete_files(&files_to_action, options.dry_run) {
                            Ok((count, logs)) => {
                                total_deleted += count;
                                // Print and log all messages
                                for log_msg in logs {
                                    log::info!("{}", log_msg);
                                    println!("{}", log_msg);
                                }
                            }
                            Err(e) => {
                                log::error!("Error during deletion batch: {}", e);
                                eprintln!("Error: {}", e);
                            }
                        }
                    } else if let Some(ref target_move_dir) = options.move_to {
                        match crate::file_utils::move_files(
                            &files_to_action,
                            target_move_dir,
                            options.dry_run,
                        ) {
                            Ok((count, logs)) => {
                                total_moved += count;
                                // Print and log all messages
                                for log_msg in logs {
                                    log::info!("{}", log_msg);
                                    println!("{}", log_msg);
                                }
                            }
                            Err(e) => {
                                log::error!("Error during move batch: {}", e);
                                eprintln!("Error: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Could not determine action targets for a set: {}", e);
                    eprintln!(
                        "Error: Could not determine which files to keep/delete: {}",
                        e
                    );
                }
            }
        }

        // Add appropriate prefix based on dry run mode
        let action_prefix = if options.dry_run {
            "[DRY RUN] Would have "
        } else {
            ""
        };

        if options.delete {
            let msg = format!("{}deleted {} files", action_prefix, total_deleted);
            log::info!("{}", msg);
            println!("\n{}", msg);
        }
        if options.move_to.is_some() {
            let msg = format!("{}moved {} files", action_prefix, total_moved);
            log::info!("{}", msg);
            println!("\n{}", msg);
        }

        // Add final reminder if in dry run mode
        if options.dry_run {
            println!("\nThis was a dry run. No files were actually modified.");
            println!("Run without --dry-run to perform actual operations.");
            log::info!("Dry run completed - no files were modified");
        }
    } else {
        log::info!("No action flags (--delete or --move-to) specified. Listing duplicates only.");
    }

    Ok(())
}

/// Run the Copy Missing mode in interactive TUI
fn run_copy_missing_interactive_mode(options: &Options) -> Result<()> {
    // Always set up file-based logging for TUI mode to avoid console disruption
    // If log_file is specified, use that, otherwise use a default location
    let log_path = options.log_file.as_ref().map(|p| p.as_path()).unwrap_or_else(|| Path::new("dedups.log"));
    
    // Make sure logging is set up before any log calls
    if let Err(e) = setup_logger(options.verbose, Some(log_path)) {
        eprintln!("Warning: Failed to set up logging: {}", e);
        // Continue anyway, just without logging
    }
    
    log::info!(
        "Interactive Copy Missing mode selected for directories: {:?}",
        options.directories
    );
    
    // Create a copy of options with progress_tui set to true to ensure proper message passing
    let mut tui_options = options.clone();
    tui_options.progress_tui = true;
    
    // Call the unified TUI function with copy_missing set to true
    tui_app::run_tui_app_for_mode(&tui_options, true)
}

/// Handle Copy Missing mode operations without TUI
fn handle_copy_missing_mode(options: &Options) -> Result<()> {
    log::info!("Copy Missing mode: Finding files to copy from source to target");
    println!("Copy Missing mode: Finding files to copy from source to target...");

    // If in dry run mode, show banner
    if options.dry_run {
        log::info!("Running in DRY RUN mode - no files will be modified");
        println!("\n===== DRY RUN MODE - NO FILES WILL BE MODIFIED =====\n");
    }

    // Determine target directory
    let target_dir = crate::file_utils::determine_target_directory(options)?;
    let source_dirs = crate::file_utils::get_source_directories(options, &target_dir);

    println!("Source directories: {:?}", source_dirs);
    println!("Target directory: {:?}", target_dir);

    // Find missing files that aren't in the target directory
    let comparison_result = crate::file_utils::compare_directories(options)?;

    // Handle missing files
    if !comparison_result.missing_in_target.is_empty() {
        println!(
            "Found {} files that exist in source but not in target.",
            comparison_result.missing_in_target.len()
        );

        // Copy missing files to target directory
        match crate::file_utils::copy_missing_files(
            &comparison_result.missing_in_target,
            &target_dir,
            options.dry_run,
        ) {
            Ok((count, logs)) => {
                // Display all log messages
                for log_msg in logs {
                    // Only log to file what hasn't already been logged in the function
                    if !log_msg.starts_with("[DRY RUN]") {
                        log::info!("{}", log_msg);
                    }
                    println!("{}", log_msg);
                }

                // Adjust the summary message based on dry run mode
                let action_prefix = if options.dry_run {
                    "[DRY RUN] Would have copied"
                } else {
                    "Successfully copied"
                };
                println!("\n{} {} files to target directory.", action_prefix, count);
            }
            Err(e) => {
                log::error!("Failed to copy files: {}", e);
                eprintln!("Error copying files: {}", e);
                return Err(anyhow::anyhow!("Failed to copy files: {}", e));
            }
        }
    } else {
        println!("No missing files found. The target directory already contains all files from the source directories.");
    }

    // Add final reminder if in dry run mode
    if options.dry_run {
        println!("\nThis was a dry run. No files were actually modified.");
        println!("Run without --dry-run to perform actual operations.");
        log::info!("Dry run completed - no files were modified");
    }

    Ok(())
} 