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
