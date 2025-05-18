// mod file_utils;
// mod tui_app;

use anyhow::Result;
use humansize::{format_size, DECIMAL};
use simplelog::LevelFilter;
use std::path::Path;
use std::str::FromStr;

use dedups::config::DedupConfig;
use dedups::file_utils::{self, is_remote_path};
use dedups::tui_app;
use dedups::Cli;

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
    // Load CLI args with config from .deduprc
    let cli = Cli::with_config()?;

    // Configure logging based on mode
    if cli.interactive {
        // For interactive mode, use a file
        let log_file = Some(Path::new("dedups.log"));
        setup_logger(cli.verbose, log_file)?;
    } else if cli.log || cli.log_file.is_some() {
        // User enabled logging
        let log_path = if let Some(path) = &cli.log_file {
            path.as_path()
        } else {
            Path::new("dedups.log")
        };
        setup_logger(cli.verbose, Some(log_path))?;
    } else if cli.json {
        // For JSON mode, completely suppress console logging to avoid corrupting the JSON output
        // We create a logger that discards all messages
        struct NullLogger;
        impl log::Log for NullLogger {
            fn enabled(&self, _: &log::Metadata) -> bool {
                false
            }
            fn log(&self, _: &log::Record) {}
            fn flush(&self) {}
        }
        let logger = Box::new(NullLogger);
        log::set_boxed_logger(logger).map(|()| log::set_max_level(log::LevelFilter::Off))?;
    } else if cli.progress {
        // CLI progress display - use terminal logger
        simplelog::TermLogger::init(
            match cli.verbose {
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
            match cli.verbose {
                0 => LevelFilter::Info,
                1 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            },
            simplelog::Config::default(),
        )?;
    }

    log::info!("Logger initialized. Application starting.");
    log::debug!("CLI args: {:#?}", cli);

    // Check if we should run in server mode
    #[cfg(feature = "ssh")]
    if cli.server_mode {
        log::info!("Starting in server mode on port {}", cli.port);
        let port = if cli.port == 0 {
            match dedups::protocol::find_available_port(10000, 20000) {
                Ok(p) => {
                    log::info!("Auto-selected port {}", p);
                    p
                }
                Err(e) => {
                    log::error!("Failed to find available port: {}", e);
                    return Err(anyhow::anyhow!("Failed to find available port: {}", e));
                }
            }
        } else {
            cli.port
        };
        return start_server_mode(port);
    }

    // Log config file path for debugging
    if let Some(config_path) = &cli.config_file {
        log::info!("Using custom config file: {:?}", config_path);
        if !config_path.exists() {
            log::warn!("Custom config file does not exist: {:?}", config_path);
            log::info!("Will use default configuration values");
        }
    } else {
        match DedupConfig::get_config_path() {
            Ok(path) => {
                log::debug!("Config file path: {:?}", path);
                if path.exists() {
                    log::debug!("Using configuration from {:?}", path);
                } else {
                    log::debug!("No config file found at {:?}, using defaults", path);
                }
            }
            Err(e) => log::warn!("Could not determine config file path: {}", e),
        }
    }

    // Check if directories exist (only for local paths)
    for dir in &cli.directories {
        if !is_remote_path(dir) {
            if !dir.exists() {
                log::error!("Local directory {:?} does not exist.", dir);
                return Err(anyhow::anyhow!("Local directory does not exist: {:?}", dir));
            }
            if !dir.is_dir() {
                log::error!("Local path {:?} is not a directory.", dir);
                return Err(anyhow::anyhow!("Local path is not a directory: {:?}", dir));
            }
        }
    }

    // Check if we're comparing multiple directories
    let is_multi_directory = cli.directories.len() > 1 || cli.target.is_some();

    if cli.interactive {
        log::info!(
            "Interactive mode selected for directories: {:?}",
            cli.directories
        );
        tui_app::run_tui_app(&cli)?
    } else if is_multi_directory {
        // Multiple directory mode - handling copying missing files or deduplication
        handle_multi_directory_mode(&cli)?;
    } else {
        // Single directory mode - find duplicates within one directory
        log::info!(
            "Non-interactive mode selected for directory: {:?}",
            cli.directories[0]
        );

        // Since we're not in TUI mode, we need a channel to receive progress updates
        let (tx, _rx) = std::sync::mpsc::channel();

        // Use the JSON-specific implementation when json flag is set
        let duplicates_result = if cli.json {
            file_utils::find_duplicate_files_with_json_progress(&cli, tx)
        } else {
            file_utils::find_duplicate_files_with_progress(&cli, tx)
        };

        match duplicates_result {
            Ok(duplicate_sets) => {
                if duplicate_sets.is_empty() {
                    log::info!("No duplicate files found.");

                    if cli.json {
                        // Output already handled by find_duplicate_files_with_json_progress
                        // No need to output anything here
                    } else {
                        println!("No duplicate files found.");
                    }
                } else {
                    // Handle duplicate sets appropriately
                    if cli.json {
                        // JSON output already handled by find_duplicate_files_with_json_progress
                        // Just process any actions (delete/move) if needed
                        if cli.delete || cli.move_to.is_some() {
                            handle_duplicate_sets(&cli, &duplicate_sets)?;
                        }
                    } else {
                        // Normal CLI output mode
                        handle_duplicate_sets(&cli, &duplicate_sets)?;
                    }
                }
            }
            Err(e) => {
                log::error!("Error finding duplicate files: {}", e);

                if cli.json {
                    // Output error as JSON
                    println!(
                        "{{\"type\":\"error\",\"message\":\"{}\",\"code\":1}}",
                        e.to_string().replace('\"', "\\\"")
                    );
                } else {
                    eprintln!("Error: {}", e);
                }
            }
        }
    }

    Ok(())
}

// Handle multiple directory mode - comparing directories and copying/deduplicating
fn handle_multi_directory_mode(cli: &Cli) -> Result<()> {
    log::info!("Multi-directory mode: Comparing directories");

    let start_time = std::time::Instant::now();

    if cli.json {
        // Send initial progress information
        let progress = file_utils::ProgressInfo {
            stage: 1,
            stage_name: "Initializing".to_string(),
            files_processed: 0,
            total_files: 0,
            percent_complete: 0.0,
            current_file: None,
            bytes_processed: 0,
            total_bytes: 0,
            elapsed_seconds: 0.0,
            estimated_seconds_left: None,
            status_message: "Comparing directories for missing files or duplicates...".to_string(),
        };

        let json_output = file_utils::JsonOutput::Progress(progress);
        if let Ok(json_str) = serde_json::to_string(&json_output) {
            println!("{}", json_str);
        }
    } else {
        println!("Comparing directories for missing files or duplicates...");
    }

    // If in dry run mode, show banner
    if cli.dry_run && !cli.json {
        log::info!("Running in DRY RUN mode - no files will be modified");
        println!("\n===== DRY RUN MODE - NO FILES WILL BE MODIFIED =====\n");
    }

    let target_dir = file_utils::determine_target_directory(cli)?;
    let source_dirs = file_utils::get_source_directories(cli, &target_dir);

    if !cli.json {
        println!("Source directories: {:?}", source_dirs);
        println!("Target directory: {:?}", target_dir);
    } else {
        // Output directory info in JSON format
        let dir_info = serde_json::json!({
            "type": "directories",
            "source_dirs": source_dirs.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<String>>(),
            "target_dir": target_dir.to_string_lossy().to_string(),
            "dry_run": cli.dry_run
        });
        println!("{}", serde_json::to_string(&dir_info)?);

        // Update progress
        let progress = file_utils::ProgressInfo {
            stage: 2,
            stage_name: "Comparing".to_string(),
            files_processed: 0,
            total_files: 0,
            percent_complete: 10.0, // Arbitrary progress indication
            current_file: None,
            bytes_processed: 0,
            total_bytes: 0,
            elapsed_seconds: start_time.elapsed().as_secs_f64(),
            estimated_seconds_left: None,
            status_message: "Comparing directories for differences...".to_string(),
        };

        let json_output = file_utils::JsonOutput::Progress(progress);
        if let Ok(json_str) = serde_json::to_string(&json_output) {
            println!("{}", json_str);
        }
    }

    let comparison_result = file_utils::compare_directories(cli)?;

    // Update progress in JSON mode
    if cli.json {
        let progress = file_utils::ProgressInfo {
            stage: 2,
            stage_name: "Comparing".to_string(),
            files_processed: 0,
            total_files: 0,
            percent_complete: 50.0, // Halfway through directory comparison
            current_file: None,
            bytes_processed: 0,
            total_bytes: 0,
            elapsed_seconds: start_time.elapsed().as_secs_f64(),
            estimated_seconds_left: None,
            status_message: "Directory comparison complete, processing results...".to_string(),
        };

        let json_output = file_utils::JsonOutput::Progress(progress);
        if let Ok(json_str) = serde_json::to_string(&json_output) {
            println!("{}", json_str);
        }
    }

    // JSON output structure for multi-directory mode
    let mut json_result = std::collections::HashMap::new();

    // Handle missing files
    if !comparison_result.missing_in_target.is_empty() {
        if !cli.json {
            println!(
                "Found {} files that exist in source but not in target.",
                comparison_result.missing_in_target.len()
            );

            if cli.deduplicate {
                println!("Deduplication mode enabled. Missing files will be considered separately from duplicates.");
            }

            if cli.delete {
                println!("Warning: Delete flag is ignored for missing files. Use --deduplicate to handle duplicates.");
            } else if cli.move_to.is_some() {
                println!("Warning: Move flag is ignored for missing files. They will be copied to the target directory.");
            }
        } else {
            // Add missing files to JSON result
            let missing_files: Vec<String> = comparison_result
                .missing_in_target
                .iter()
                .map(|f| f.path.display().to_string())
                .collect();

            // Stream the missing files information
            let missing_info = serde_json::json!({
                "type": "missing_files",
                "count": missing_files.len(),
                "files": missing_files
            });
            println!("{}", serde_json::to_string(&missing_info)?);

            // Still add to the final result
            json_result.insert(
                "missing_files".to_string(),
                serde_json::json!(missing_files),
            );
            json_result.insert(
                "missing_count".to_string(),
                serde_json::json!(missing_files.len()),
            );
        }

        // Update progress in JSON mode before copying
        if cli.json {
            let progress = file_utils::ProgressInfo {
                stage: 3,
                stage_name: "Copying".to_string(),
                files_processed: 0,
                total_files: comparison_result.missing_in_target.len(),
                percent_complete: 0.0,
                current_file: None,
                bytes_processed: 0,
                total_bytes: comparison_result
                    .missing_in_target
                    .iter()
                    .map(|f| f.size)
                    .sum(),
                elapsed_seconds: start_time.elapsed().as_secs_f64(),
                estimated_seconds_left: None,
                status_message: format!(
                    "Preparing to copy {} missing files",
                    comparison_result.missing_in_target.len()
                ),
            };

            let json_output = file_utils::JsonOutput::Progress(progress);
            if let Ok(json_str) = serde_json::to_string(&json_output) {
                println!("{}", json_str);
            }
        }

        // Copy missing files to target directory
        match file_utils::copy_missing_files(
            &comparison_result.missing_in_target,
            &target_dir,
            cli.dry_run,
        ) {
            Ok((count, logs)) => {
                // Store logs for JSON output if needed
                let mut operation_logs = Vec::new();

                // Display all log messages
                for log_msg in logs {
                    // Only log to file what hasn't already been logged in the function
                    if !log_msg.starts_with("[DRY RUN]") {
                        log::info!("{}", log_msg);
                    }

                    if !cli.json {
                        println!("{}", log_msg);
                    } else {
                        operation_logs.push(log_msg);
                    }
                }

                // Adjust the summary message based on dry run mode
                if !cli.json {
                    let action_prefix = if cli.dry_run {
                        "[DRY RUN] Would have copied"
                    } else {
                        "Successfully copied"
                    };
                    println!("\n{} {} files to target directory.", action_prefix, count);
                } else {
                    // Stream the copy operation results
                    let copy_result = serde_json::json!({
                        "type": "copy_result",
                        "copied_count": count,
                        "dry_run": cli.dry_run,
                        "logs": operation_logs,
                    });
                    println!("{}", serde_json::to_string(&copy_result)?);

                    // Add copy operation results to overall JSON
                    json_result.insert("copied_count".to_string(), serde_json::json!(count));
                    json_result.insert("dry_run".to_string(), serde_json::json!(cli.dry_run));
                    json_result.insert(
                        "operation_logs".to_string(),
                        serde_json::json!(operation_logs),
                    );
                }
            }
            Err(e) => {
                log::error!("Failed to copy files: {}", e);

                if !cli.json {
                    eprintln!("Error copying files: {}", e);
                } else {
                    // Stream the error
                    let error = serde_json::json!({
                        "type": "error",
                        "message": e.to_string(),
                        "code": 2,
                        "operation": "copy_files"
                    });
                    println!("{}", serde_json::to_string(&error)?);

                    // Add to overall result
                    json_result.insert("error".to_string(), serde_json::json!(e.to_string()));
                }
            }
        }
    } else if !cli.json {
        println!("No missing files found in target directory.");
    } else {
        // Stream the empty missing files result
        let missing_info = serde_json::json!({
            "type": "missing_files",
            "count": 0,
            "files": []
        });
        println!("{}", serde_json::to_string(&missing_info)?);

        // Add to overall result
        json_result.insert(
            "missing_files".to_string(),
            serde_json::json!(Vec::<String>::new()),
        );
        json_result.insert("missing_count".to_string(), serde_json::json!(0));
    }

    // Handle duplicates if deduplication is enabled
    if cli.deduplicate && !comparison_result.duplicates.is_empty() {
        if !cli.json {
            println!(
                "Found {} duplicate sets across source and target directories.",
                comparison_result.duplicates.len()
            );
        } else {
            // Stream duplicate count info
            let dup_info = serde_json::json!({
                "type": "duplicate_info",
                "count": comparison_result.duplicates.len(),
                "total_files": comparison_result.duplicates.iter().map(|set| set.files.len()).sum::<usize>()
            });
            println!("{}", serde_json::to_string(&dup_info)?);
        }

        // Process duplicates similar to single directory mode
        let duplicate_json = handle_duplicate_sets(cli, &comparison_result.duplicates)?;

        // If JSON was returned, merge it into our results
        if cli.json && duplicate_json.is_some() {
            if let Some(dup_json) = duplicate_json {
                json_result.insert("duplicates".to_string(), dup_json);
            }
        }
    } else if cli.deduplicate {
        if !cli.json {
            println!("No duplicate files found across source and target directories.");
        } else {
            // Stream the empty duplicates result
            let dup_info = serde_json::json!({
                "type": "duplicate_info",
                "count": 0,
                "total_files": 0
            });
            println!("{}", serde_json::to_string(&dup_info)?);

            // Add to overall result
            json_result.insert("duplicates".to_string(), serde_json::json!({}));
        }
    }

    // Add final reminder if in dry run mode
    if cli.dry_run && !cli.json {
        println!("\nThis was a dry run. No files were actually modified.");
        println!("Run without --dry-run to perform actual operations.");
        log::info!("Dry run completed - no files were modified");
    }

    // Output final JSON result summary if needed
    if cli.json {
        // Add elapsed time
        json_result.insert(
            "elapsed_seconds".to_string(),
            serde_json::json!(start_time.elapsed().as_secs_f64()),
        );

        // Final result summary
        let result = serde_json::json!({
            "type": "final_result",
            "dry_run": cli.dry_run,
            "elapsed_seconds": start_time.elapsed().as_secs_f64(),
            "result": json_result
        });
        println!("{}", serde_json::to_string(&result)?);
    }

    Ok(())
}

// Handle duplicate sets (common code for both single and multi-directory modes)
fn handle_duplicate_sets(
    cli: &Cli,
    duplicate_sets: &[file_utils::DuplicateSet],
) -> Result<Option<serde_json::Value>> {
    log::info!("Found {} sets of duplicate files.", duplicate_sets.len());

    // Initialize JSON structure for duplicate sets if needed
    let mut json_duplicate_sets = std::collections::HashMap::new();

    if !cli.json {
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
    } else {
        // Build JSON structure for duplicate sets
        for (idx, set) in duplicate_sets.iter().enumerate() {
            let mut set_json = std::collections::HashMap::new();
            set_json.insert("count".to_string(), serde_json::json!(set.files.len()));
            set_json.insert("size".to_string(), serde_json::json!(set.size));
            set_json.insert(
                "size_human".to_string(),
                serde_json::json!(format_size(set.size, DECIMAL)),
            );
            set_json.insert("hash".to_string(), serde_json::json!(set.hash.clone()));

            let file_paths: Vec<String> = set
                .files
                .iter()
                .map(|f| f.path.display().to_string())
                .collect();
            set_json.insert("files".to_string(), serde_json::json!(file_paths));

            json_duplicate_sets.insert(format!("set_{}", idx + 1), serde_json::json!(set_json));
        }

        // Print JSON directly in this function since we're now handling JSON output completely inside handle_duplicate_sets
        if cli.json {
            // Format and print the JSON output
            let json_output = serde_json::to_string_pretty(&json_duplicate_sets)?;
            println!("{}", json_output);
        }
    }

    if let Some(output_path) = &cli.output {
        match file_utils::output_duplicates(duplicate_sets, output_path, &cli.format) {
            Ok(_) => {
                log::info!("Successfully wrote duplicate list to {:?}", output_path);

                if !cli.json {
                    println!("Duplicate list saved to {:?}", output_path);
                } else {
                    json_duplicate_sets.insert(
                        "output_file".to_string(),
                        serde_json::json!(output_path.display().to_string()),
                    );
                }
            }
            Err(e) => {
                log::error!("Failed to write duplicate list to {:?}: {}", output_path, e);

                if !cli.json {
                    eprintln!("Failed to write output file: {}", e);
                } else {
                    json_duplicate_sets
                        .insert("output_error".to_string(), serde_json::json!(e.to_string()));
                }
            }
        }
    }

    if cli.delete || cli.move_to.is_some() {
        // Add action results to JSON
        let mut action_results = std::collections::HashMap::new();

        // Log dry run mode status at the beginning
        if cli.dry_run && !cli.json {
            log::info!("Running in DRY RUN mode - no files will be modified");
            println!("\n===== DRY RUN MODE - NO FILES WILL BE MODIFIED =====\n");
        }

        let strategy = file_utils::SelectionStrategy::from_str(&cli.mode)?;
        let mut total_deleted = 0;
        let mut total_moved = 0;
        let mut all_logs = Vec::new();

        for set in duplicate_sets {
            if set.files.len() < 2 {
                continue;
            }

            match file_utils::determine_action_targets(set, strategy) {
                Ok((kept_file, files_to_action)) => {
                    log::info!(
                        "For duplicate set (hash: {}...), keeping file: {:?}",
                        set.hash.chars().take(8).collect::<String>(),
                        kept_file.path
                    );

                    if !cli.json {
                        println!("Keeping: {}", kept_file.path.display());
                    }

                    if cli.delete {
                        match file_utils::delete_files(&files_to_action, cli.dry_run) {
                            Ok((count, logs)) => {
                                total_deleted += count;
                                // Store logs for JSON output
                                all_logs.extend(logs.clone());

                                // Print logs if not in JSON mode
                                if !cli.json {
                                    for log_msg in logs {
                                        log::info!("{}", log_msg);
                                        println!("{}", log_msg);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Error during deletion batch: {}", e);

                                if !cli.json {
                                    eprintln!("Error: {}", e);
                                } else {
                                    action_results.insert(
                                        "delete_error".to_string(),
                                        serde_json::json!(e.to_string()),
                                    );
                                }
                            }
                        }
                    } else if let Some(ref target_move_dir) = cli.move_to {
                        match file_utils::move_files(&files_to_action, target_move_dir, cli.dry_run)
                        {
                            Ok((count, logs)) => {
                                total_moved += count;
                                // Store logs for JSON output
                                all_logs.extend(logs.clone());

                                // Print logs if not in JSON mode
                                if !cli.json {
                                    for log_msg in logs {
                                        log::info!("{}", log_msg);
                                        println!("{}", log_msg);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Error during move batch: {}", e);

                                if !cli.json {
                                    eprintln!("Error: {}", e);
                                } else {
                                    action_results.insert(
                                        "move_error".to_string(),
                                        serde_json::json!(e.to_string()),
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Could not determine action targets for a set: {}", e);

                    if !cli.json {
                        eprintln!(
                            "Error: Could not determine which files to keep/delete: {}",
                            e
                        );
                    } else {
                        action_results
                            .insert("action_error".to_string(), serde_json::json!(e.to_string()));
                    }
                }
            }
        }

        // Add appropriate prefix based on dry run mode
        let action_prefix = if cli.dry_run {
            "[DRY RUN] Would have "
        } else {
            ""
        };

        if cli.delete {
            let msg = format!("{}deleted {} files", action_prefix, total_deleted);
            log::info!("{}", msg);

            if !cli.json {
                println!("\n{}", msg);
            } else {
                action_results.insert(
                    "deleted_count".to_string(),
                    serde_json::json!(total_deleted),
                );
                action_results.insert("dry_run".to_string(), serde_json::json!(cli.dry_run));
            }
        }
        if cli.move_to.is_some() {
            let msg = format!("{}moved {} files", action_prefix, total_moved);
            log::info!("{}", msg);

            if !cli.json {
                println!("\n{}", msg);
            } else {
                action_results.insert("moved_count".to_string(), serde_json::json!(total_moved));
                action_results.insert(
                    "move_target".to_string(),
                    serde_json::json!(cli.move_to.as_ref().unwrap().display().to_string()),
                );
                action_results.insert("dry_run".to_string(), serde_json::json!(cli.dry_run));
            }
        }

        // Add logs to action results
        if cli.json {
            action_results.insert("logs".to_string(), serde_json::json!(all_logs));
            json_duplicate_sets.insert("actions".to_string(), serde_json::json!(action_results));
        }

        // Add final reminder if in dry run mode
        if cli.dry_run && !cli.json {
            println!("\nThis was a dry run. No files were actually modified.");
            println!("Run without --dry-run to perform actual operations.");
            log::info!("Dry run completed - no files were modified");
        }
    } else if !cli.json {
        log::info!("No action flags (--delete or --move-to) specified. Listing duplicates only.");
    }

    // If in JSON mode, return the JSON structure, otherwise return None
    if cli.json {
        Ok(Some(serde_json::json!(json_duplicate_sets)))
    } else {
        Ok(None)
    }
}

// Start server mode to handle commands from remote clients
#[cfg(feature = "ssh")]
fn start_server_mode(port: u16) -> Result<()> {
    use dedups::server::run_server;

    log::info!("Starting dedups server on port {}", port);

    // Print the port so the client can connect
    if std::env::var("RUST_LOG").is_ok() {
        println!("DEDUPS_SERVER_PORT={}", port);
    }

    // Run the server
    run_server(port)?;

    Ok(())
}
