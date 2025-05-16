// mod file_utils;
// mod tui_app;

use std::path::Path;
use clap::Parser;
use simplelog::LevelFilter;
use anyhow::Result;
use humansize::{format_size, DECIMAL};
use env_logger;

use dedup_tui::Cli;
use dedup_tui::file_utils;
use dedup_tui::tui_app;

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

    let dedup_tui_log = std::path::PathBuf::from("dedup_tui.log");
    let dedup_log = std::path::PathBuf::from("dedup.log");
    let log_file = if cli.interactive {
        Some(dedup_tui_log.as_path())
    } else if cli.log {
        Some(dedup_log.as_path())
    } else {
        None
    };
    setup_logger(cli.verbose, log_file)?;

    log::info!("Logger initialized. Application starting.");
    log::debug!("CLI args: {:#?}", cli);

    // Check if directories exist
    for dir in &cli.directories {
        if !dir.exists() {
            log::error!("Target directory {:?} does not exist.", dir);
            return Err(anyhow::anyhow!("Target directory does not exist: {:?}", dir));
        }
        if !dir.is_dir() {
            log::error!("Target path {:?} is not a directory.", dir);
            return Err(anyhow::anyhow!("Target path is not a directory: {:?}", dir));
        }
    }

    // Check if we're comparing multiple directories
    let is_multi_directory = cli.directories.len() > 1 || cli.target.is_some();

    if cli.interactive {
        log::info!("Interactive mode selected for directories: {:?}", cli.directories);
        tui_app::run_tui_app(&cli)?
    } else if is_multi_directory {
        // Multiple directory mode - handling copying missing files or deduplication
        handle_multi_directory_mode(&cli)?;
    } else {
        // Single directory mode - find duplicates within one directory
        log::info!("Non-interactive mode selected for directory: {:?}", cli.directories[0]);
        match file_utils::find_duplicate_files(&cli) {
            Ok(duplicate_sets) => {
                if duplicate_sets.is_empty() {
                    log::info!("No duplicate files found.");
                    println!("No duplicate files found.");
                } else {
                    handle_duplicate_sets(&cli, &duplicate_sets)?;
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

// Handle multiple directory mode - comparing directories and copying/deduplicating
fn handle_multi_directory_mode(cli: &Cli) -> Result<()> {
    log::info!("Multi-directory mode: Comparing directories");
    println!("Comparing directories for missing files or duplicates...");

    let target_dir = file_utils::determine_target_directory(cli)?;
    let source_dirs = file_utils::get_source_directories(cli, &target_dir);
    
    println!("Source directories: {:?}", source_dirs);
    println!("Target directory: {:?}", target_dir);
    
    let comparison_result = file_utils::compare_directories(cli)?;
    
    // Handle missing files
    if !comparison_result.missing_in_target.is_empty() {
        println!("Found {} files that exist in source but not in target.", 
                 comparison_result.missing_in_target.len());
        
        if cli.deduplicate {
            println!("Deduplication mode enabled. Missing files will be considered separately from duplicates.");
        }
        
        if cli.delete {
            println!("Warning: Delete flag is ignored for missing files. Use --deduplicate to handle duplicates.");
        } else if cli.move_to.is_some() {
            println!("Warning: Move flag is ignored for missing files. They will be copied to the target directory.");
        }
        
        // Copy missing files to target directory
        let copy_count = file_utils::copy_missing_files(
            &comparison_result.missing_in_target, 
            &target_dir, 
            false // Not dry run
        )?;
        
        println!("Successfully copied {} files to target directory.", copy_count);
    } else {
        println!("No missing files found in target directory.");
    }
    
    // Handle duplicates if deduplication is enabled
    if cli.deduplicate && !comparison_result.duplicates.is_empty() {
        println!("Found {} duplicate sets across source and target directories.", 
                 comparison_result.duplicates.len());
        
        // Process duplicates similar to single directory mode
        handle_duplicate_sets(cli, &comparison_result.duplicates)?;
    } else if cli.deduplicate {
        println!("No duplicate files found across source and target directories.");
    }
    
    Ok(())
}

// Handle duplicate sets (common code for both single and multi-directory modes)
fn handle_duplicate_sets(cli: &Cli, duplicate_sets: &[file_utils::DuplicateSet]) -> Result<()> {
    log::info!("Found {} sets of duplicate files.", duplicate_sets.len());
    println!("Found {} sets of duplicate files:", duplicate_sets.len());
    
    for set in duplicate_sets {
        println!("  Duplicates ({} files, size: {}, hash: {}...):", 
                 set.files.len(), 
                 format_size(set.size, DECIMAL),
                 set.hash.chars().take(16).collect::<String>());
        for file_info in &set.files {
            println!("    - {}", file_info.path.display());
        }
    }

    if let Some(output_path) = &cli.output {
        match file_utils::output_duplicates(duplicate_sets, output_path, &cli.format) {
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
    
    if cli.delete || cli.move_to.is_some() {
        let strategy = file_utils::SelectionStrategy::from_str(&cli.mode)?;
        let mut total_deleted = 0;
        let mut total_moved = 0;

        for set in duplicate_sets {
            if set.files.len() < 2 { continue; }

            match file_utils::determine_action_targets(set, strategy) {
                Ok((kept_file, files_to_action)) => {
                    log::info!("For duplicate set (hash: {}...), keeping file: {:?}", 
                             set.hash.chars().take(8).collect::<String>(), 
                             kept_file.path);
                    println!("Keeping: {}", kept_file.path.display());

                    if cli.delete {
                        match file_utils::delete_files(&files_to_action, cli.progress) {
                            Ok((count, _logs)) => total_deleted += count,
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
    
    Ok(())
}
