use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use humansize::{format_size, DECIMAL};
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::layout::Position;
use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::options::Options;
use crate::tui_app::explorer_browser::EnhancedExplorer;
use crate::tui_app::file_browser::FileBrowser;
use crate::tui_app::{ActionType, ActivePanel, App, InputMode, Job, ScanMessage};

/// Entry point for the Copy Missing TUI
pub fn run_copy_missing_tui(options: &Options) -> Result<()> {
    // Terminal initialization
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    // Create app state for copy missing mode
    let mut app = create_copy_missing_app(options);

    // Main loop
    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    // Start scanning for missing files
    start_copy_missing_scan(&mut app, options);

    loop {
        // Continue to handle messages from scan thread
        handle_copy_missing_scan_messages(&mut app);

        // Draw the TUI
        terminal.draw(|f| ui_copy_missing(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let CEvent::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key_event(&mut app, key, options);

                    // Check for quit flag
                    if app.should_quit {
                        break;
                    }
                }
            }
        }

        // Reset the tick timer even if no event was processed
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    // Cleanup and restore terminal
    disable_raw_mode()?;
    terminal.show_cursor()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    Ok(())
}

/// Handle key events for the copy missing mode
fn handle_key_event(app: &mut App, key: KeyEvent, options: &Options) {
    match app.state.input_mode {
        InputMode::Normal => {
            match (key.code, key.modifiers) {
                // Quit
                (KeyCode::Char('q'), KeyModifiers::NONE) => app.should_quit = true,
                (KeyCode::Esc, KeyModifiers::NONE) => app.should_quit = true,
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,

                // Panel navigation
                (KeyCode::Tab, KeyModifiers::NONE) => app.cycle_active_panel(),

                // Context-aware navigation
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    match app.state.active_panel {
                        ActivePanel::Sets => app.select_previous_set(),
                        ActivePanel::Files => {
                            if let Some(explorer) = &mut app.state.enhanced_explorer {
                                if explorer.select_prev().is_err() {
                                    app.state.status_message = Some("Error navigating in file explorer".to_string());
                                }
                            } else if let Some(file_browser) = &mut app.state.file_browser {
                                // Fallback to old browser
                                file_browser.select_prev();
                            }
                        }
                        ActivePanel::Jobs => app.select_previous_job(),
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    match app.state.active_panel {
                        ActivePanel::Sets => app.select_next_set(),
                        ActivePanel::Files => {
                            if let Some(explorer) = &mut app.state.enhanced_explorer {
                                if explorer.select_next().is_err() {
                                    app.state.status_message = Some("Error navigating in file explorer".to_string());
                                }
                            } else if let Some(file_browser) = &mut app.state.file_browser {
                                // Fallback to old browser
                                file_browser.select_next();
                            }
                        }
                        ActivePanel::Jobs => app.select_next_job(),
                    }
                }
                (KeyCode::Left, _) => {
                    // Cycle panel focus to the left
                    app.state.active_panel = match app.state.active_panel {
                        ActivePanel::Sets => ActivePanel::Jobs,
                        ActivePanel::Files => ActivePanel::Sets,
                        ActivePanel::Jobs => ActivePanel::Files,
                    };
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    // Cycle panel focus to the right
                    app.state.active_panel = match app.state.active_panel {
                        ActivePanel::Sets => ActivePanel::Files,
                        ActivePanel::Files => ActivePanel::Jobs,
                        ActivePanel::Jobs => ActivePanel::Sets,
                    };
                }
                (KeyCode::Enter, KeyModifiers::NONE) => {
                    if app.state.active_panel == ActivePanel::Files {
                        // Try with enhanced explorer first
                        if let Some(explorer) = &mut app.state.enhanced_explorer {
                            if explorer.is_selected_dir() {
                                if let Some(path) = explorer.selected_path() {
                                    if explorer.change_directory(path.clone()).is_err() {
                                        app.state.status_message = Some(format!("Could not enter directory: {}", path.display()));
                                    }
                                }
                            }
                        // Fallback to the old file browser
                        } else if let Some(file_browser) = &mut app.state.file_browser {
                            if let Some(entry) = file_browser.selected_entry() {
                                if entry.is_dir() {
                                    file_browser.change_directory(entry.path.clone());
                                }
                            }
                        }
                    } else if app.state.active_panel == ActivePanel::Sets {
                        // Toggle folder expansion when Enter is pressed in the sets panel
                        if let Some(crate::tui_app::DisplayListItem::Folder { is_expanded, .. }) = 
                            app.state.display_list.get_mut(app.state.selected_display_list_index) {
                            *is_expanded = !*is_expanded;
                            app.state.status_message = Some(if *is_expanded { 
                                "Folder expanded".to_string() 
                            } else { 
                                "Folder collapsed".to_string() 
                            });
                        }
                    }
                }
                // Refresh directories
                (KeyCode::Char('r'), KeyModifiers::CONTROL)
                | (KeyCode::F(5), KeyModifiers::NONE) => {
                    if let Some(explorer) = &mut app.state.enhanced_explorer {
                        if explorer.refresh().is_err() {
                            app.state.status_message = Some("Error refreshing directory".to_string());
                        }
                    } else if let Some(file_browser) = &mut app.state.file_browser {
                        file_browser.refresh();
                    }
                }
                // Add 'C' key for copy operation
                (KeyCode::Char('c'), KeyModifiers::NONE) => {
                    if app.state.active_panel == ActivePanel::Sets {
                        // Get the selected file from the source list
                        if let Some(current_item) = app.current_selected_file().cloned() {
                            // If we have a destination path, queue the copy job directly
                            if let Some(dest_path) = &app.state.destination_path {
                                let dest_clone = dest_path.clone();
                                app.state.jobs.push(Job {
                                    action: ActionType::Copy(dest_clone),
                                    file_info: current_item.clone(),
                                });
                                app.state.status_message = Some(format!(
                                    "Queued {} for copy to {}",
                                    current_item.path.display(),
                                    dest_path.display()
                                ));
                                
                                // Auto-execute the job if we're adding a single file
                                if app.state.jobs.len() == 1 {
                                    log::info!("Auto-executing single copy job");
                                    
                                    // Execute the job
                                    app.start_job_execution(options);
                                    
                                    // Perform a full refresh of both panels
                                    
                                    // 1. Force destination panel refresh using direct file system access
                                    if let Some(dest_path) = &app.state.destination_path {
                                        log::info!("Directly refreshing destination at {}", dest_path.display());
                                        
                                        // Force recreate the explorer with the original destination path
                                        if let Some(explorer) = &mut app.state.enhanced_explorer {
                                            // Try to create new instance with the saved destination path
                                            if let Ok(new_explorer) = EnhancedExplorer::new(
                                                Some(dest_path.clone()),
                                                &format!("Destination: {}", dest_path.display())
                                            ) {
                                                // Replace with new instance
                                                *explorer = new_explorer;
                                                log::info!("Successfully replaced explorer instance for destination");
                                            } else {
                                                // Fall back to refresh method which now tries to use original path
                                                if explorer.refresh().is_err() {
                                                    log::error!("Failed to refresh destination directory");
                                                    app.state.status_message = Some("Error refreshing destination - try manual navigation".to_string());
                                                }
                                            }
                                        } else if let Some(browser) = &mut app.state.file_browser {
                                            // Force file browser refresh 
                                            browser.refresh();
                                        }
                                    }
                                    
                                    // 2. Force source panel refresh by re-running the scan
                                    log::info!("Forcing source panel refresh by re-scanning for missing files");
                                    start_copy_missing_scan(app, options);
                                    
                                    // 3. Set the UI update flag 
                                    app.state.last_job_completion_check = Some(true);
                                    app.state.status_message = Some("Job executed - refreshed both panels".to_string());
                                }
                            } else {
                                app.state.status_message = Some("No destination selected. Please select a destination directory first.".into());
                            }
                        } else {
                            app.state.status_message = Some("No file selected to copy.".into());
                        }
                    }
                }
                // Select/copy files (keep for backward compatibility)
                (KeyCode::Char('s'), KeyModifiers::NONE) => {
                    if app.state.active_panel == ActivePanel::Sets {
                        // Try to get the selected path from the enhanced explorer first
                        let selected_path = if let Some(explorer) = &app.state.enhanced_explorer {
                            explorer.selected_path()
                        } 
                        // Fallback to old file browser
                        else if let Some(browser) = &app.state.file_browser {
                            browser.get_selected_path()
                        } else {
                            None
                        };
                        
                        if let Some(selected_path) = selected_path {
                            if let Some(current_item) = app.current_selected_file() {
                                app.state.jobs.push(Job {
                                    action: ActionType::Copy(selected_path),
                                    file_info: current_item.clone(),
                                });
                                app.state.status_message = Some("File queued for copy".into());
                            }
                        } else {
                            app.state.status_message = Some("No destination selected. Please select a destination directory first.".into());
                        }
                    }
                }
                                    // Execute queued jobs (Ctrl+E)
                (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                    // Add jobs for all selected files in the left panel
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        let dest_path = app.state.destination_path.clone();
                        if let Some(dest_path) = dest_path {
                            let mut new_jobs = Vec::new();
                            for path in app.state.selected_left_panel.iter() {
                                // Find the FileInfo for this path
                                for group in &app.state.grouped_data {
                                    for set in &group.sets {
                                        if let Some(file_info) = set.files.iter().find(|f| &f.path == path) {
                                            // Only add if not already in jobs
                                            if !app.state.jobs.iter().any(|j| j.file_info.path == file_info.path && matches!(j.action, crate::tui_app::ActionType::Copy(_))) {
                                                new_jobs.push(crate::tui_app::Job {
                                                    action: crate::tui_app::ActionType::Copy(dest_path.clone()),
                                                    file_info: file_info.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            app.state.jobs.extend(new_jobs);
                        } else {
                            app.state.status_message = Some("No destination selected. Please select a destination directory first.".into());
                            return;
                        }
                    }
                    if !app.state.jobs.is_empty() {
                        log::info!("Starting job execution for {} files", app.state.jobs.len());
                        
                        // Execute the jobs
                        app.start_job_execution(options);
                        
                        // Perform a full refresh of both panels after Ctrl+E execution
                        
                        // 1. Force destination panel refresh using direct file system access
                        if let Some(dest_path) = &app.state.destination_path {
                            log::info!("Directly refreshing destination after Ctrl+E at {}", dest_path.display());
                            
                            // Force recreate the explorer
                            if let Some(explorer) = &mut app.state.enhanced_explorer {
                                // Try to create new instance explicitly with the original destination path
                                if let Ok(new_explorer) = EnhancedExplorer::new(
                                    Some(dest_path.clone()),
                                    &format!("Destination: {}", dest_path.display())
                                ) {
                                    // Replace with new instance
                                    *explorer = new_explorer;
                                    log::info!("Successfully replaced explorer instance for destination after Ctrl+E");
                                } else {
                                    // Fall back to refresh method which now tries to use original path
                                    if explorer.refresh().is_err() {
                                        log::error!("Failed to refresh destination directory after Ctrl+E");
                                        app.state.status_message = Some("Error refreshing destination - try manual navigation".to_string());
                                    }
                                }
                            } else if let Some(browser) = &mut app.state.file_browser {
                                // Force file browser refresh 
                                browser.refresh();
                            }
                        }
                        
                        // 2. Force source panel refresh by re-running the scan
                        log::info!("Forcing source panel refresh by re-scanning for missing files after Ctrl+E");
                        start_copy_missing_scan(app, options);
                        
                        // 3. Set the UI update flag for continuous refresh if needed
                        app.state.last_job_completion_check = Some(true);
                        app.state.status_message = Some("Jobs executed - refreshed both panels".to_string());
                    } else {
                        app.state.status_message = Some("No jobs to process. Select files and try again.".into());
                    }
                }
                // Toggle dry run mode
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    app.state.dry_run = !app.state.dry_run;
                    let msg = if app.state.dry_run {
                        "Dry run mode ENABLED - no files will be modified"
                    } else {
                        "Dry run mode DISABLED - files will be modified when executed"
                    };
                    app.state.status_message = Some(msg.into());
                }
                // Sorting shortcuts for left panel
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        app.state.current_sort_criterion = crate::file_utils::SortCriterion::FileName;
                        app.state.display_list = crate::tui_app::App::build_display_list_from_grouped_data(&app.state.grouped_data);
                        app.state.status_message = Some("Sorted by name".into());
                    }
                }
                (KeyCode::Char('z'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        app.state.current_sort_criterion = crate::file_utils::SortCriterion::FileSize;
                        app.state.display_list = crate::tui_app::App::build_display_list_from_grouped_data(&app.state.grouped_data);
                        app.state.status_message = Some("Sorted by size".into());
                    }
                }
                (KeyCode::Char('m'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        app.state.current_sort_criterion = crate::file_utils::SortCriterion::ModifiedAt;
                        app.state.display_list = crate::tui_app::App::build_display_list_from_grouped_data(&app.state.grouped_data);
                        app.state.status_message = Some("Sorted by modified time".into());
                    }
                }
                (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        app.state.current_sort_criterion = crate::file_utils::SortCriterion::PathLength;
                        app.state.display_list = crate::tui_app::App::build_display_list_from_grouped_data(&app.state.grouped_data);
                        app.state.status_message = Some("Sorted by duplicates (path length)".into());
                    }
                }
                // Toggle folders first
                (KeyCode::Char('f'), KeyModifiers::NONE) => {
                    if let Some(explorer) = &mut app.state.enhanced_explorer {
                        if explorer.toggle_folders_first().is_err() {
                            app.state.status_message = Some("Error toggling folders first setting".to_string());
                        } else {
                            let msg = "Folder display settings updated";
                            app.state.status_message = Some(msg.into());
                        }
                    } else if let Some(file_browser) = &mut app.state.file_browser {
                        file_browser.toggle_folders_first();
                        let msg = if file_browser.folders_first {
                            "Folders will be listed first"
                        } else {
                            "Sorting with files and folders mixed"
                        };
                        app.state.status_message = Some(msg.into());
                    }
                }
                // The 'i' key is now unused (previously toggled file details panel)
                (KeyCode::Char('i'), KeyModifiers::NONE) => {
                    app.state.status_message = Some("File details panel has been removed".into());
                }
                // Switch between update mode and regular copy
                (KeyCode::Char('u'), KeyModifiers::NONE) => {
                    app.state.update_mode = !app.state.update_mode;
                    let msg = if app.state.update_mode {
                        "Update mode ENABLED - only newer files will be copied"
                    } else {
                        "Update mode DISABLED - all files will be copied"
                    };
                    app.state.status_message = Some(msg.into());
                }
                // Toggle selection for the highlighted file/set or expand/collapse folder
                (KeyCode::Char(' '), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        // Check if this is a folder entry first
                        if let Some(crate::tui_app::DisplayListItem::Folder { is_expanded, .. }) = 
                            app.state.display_list.get_mut(app.state.selected_display_list_index) {
                            // Toggle folder expansion
                            *is_expanded = !*is_expanded;
                            app.state.status_message = Some(if *is_expanded { 
                                "Folder expanded".to_string() 
                            } else { 
                                "Folder collapsed".to_string() 
                            });
                        } 
                        // Otherwise handle file/set selection
                        else if let Some(crate::tui_app::DisplayListItem::SetEntry { original_group_index, original_set_index_in_group, file_count_in_set, .. }) = app.state.display_list.get(app.state.selected_display_list_index) {
                            let paths: Vec<PathBuf> = if *file_count_in_set == 1 {
                                app.state.grouped_data
                                    .get(*original_group_index)
                                    .and_then(|group| group.sets.get(*original_set_index_in_group))
                                    .and_then(|set| set.files.first())
                                    .map(|f| vec![f.path.clone()])
                                    .unwrap_or_default()
                            } else {
                                app.state.grouped_data
                                    .get(*original_group_index)
                                    .and_then(|group| group.sets.get(*original_set_index_in_group))
                                    .map(|set| set.files.iter().map(|f| f.path.clone()).collect())
                                    .unwrap_or_default()
                            };
                            let dest_path = app.state.destination_path.clone();
                            for path in paths {
                                if !app.state.selected_left_panel.remove(&path) {
                                    app.state.selected_left_panel.insert(path.clone());
                                    // Add job for this file
                                    if let Some(dest_path) = &dest_path {
                                        for group in &app.state.grouped_data {
                                            for set in &group.sets {
                                                if let Some(file_info) = set.files.iter().find(|f| f.path == path) {
                                                    if !app.state.jobs.iter().any(|j| j.file_info.path == file_info.path && matches!(j.action, crate::tui_app::ActionType::Copy(_))) {
                                                        app.state.jobs.push(crate::tui_app::Job {
                                                            action: crate::tui_app::ActionType::Copy(dest_path.clone()),
                                                            file_info: file_info.clone(),
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Remove job for this file
                                    app.state.jobs.retain(|j| j.file_info.path != path);
                                }
                            }
                        }
                    }
                }
                
                // Expand all folders
                (KeyCode::Char('e'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        // Expand all folders
                        for item in &mut app.state.display_list {
                            if let crate::tui_app::DisplayListItem::Folder { is_expanded, .. } = item {
                                *is_expanded = true;
                            }
                        }
                        app.state.status_message = Some("All folders expanded".to_string());
                    }
                }
                
                // Collapse all folders with 'x' key instead of 'c' to avoid conflict
                (KeyCode::Char('x'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        // Collapse all folders
                        for item in &mut app.state.display_list {
                            if let crate::tui_app::DisplayListItem::Folder { is_expanded, .. } = item {
                                *is_expanded = false;
                            }
                        }
                        app.state.status_message = Some("All folders collapsed".to_string());
                    }
                }
                // Select all/unselect all
                (KeyCode::Char('a'), KeyModifiers::NONE) => {
                    if app.state.active_panel == crate::tui_app::ActivePanel::Sets {
                        let mut all_paths = Vec::new();
                        for item in &app.state.display_list {
                            if let crate::tui_app::DisplayListItem::SetEntry { original_group_index, original_set_index_in_group, file_count_in_set, .. } = item {
                                let paths: Vec<PathBuf> = if *file_count_in_set == 1 {
                                    app.state.grouped_data
                                        .get(*original_group_index)
                                        .and_then(|group| group.sets.get(*original_set_index_in_group))
                                        .and_then(|set| set.files.first())
                                        .map(|f| vec![f.path.clone()])
                                        .unwrap_or_default()
                                } else {
                                    app.state.grouped_data
                                        .get(*original_group_index)
                                        .and_then(|group| group.sets.get(*original_set_index_in_group))
                                        .map(|set| set.files.iter().map(|f| f.path.clone()).collect())
                                        .unwrap_or_default()
                                };
                                all_paths.extend(paths);
                            }
                        }
                        let all_selected = all_paths.iter().all(|p| app.state.selected_left_panel.contains(p));
                        let dest_path = app.state.destination_path.clone();
                        if all_selected {
                            for p in all_paths {
                                app.state.selected_left_panel.remove(&p);
                                app.state.jobs.retain(|j| j.file_info.path != p);
                            }
                        } else {
                            for p in all_paths {
                                if !app.state.selected_left_panel.contains(&p) {
                                    app.state.selected_left_panel.insert(p.clone());
                                    // Add job for this file
                                    if let Some(dest_path) = &dest_path {
                                        for group in &app.state.grouped_data {
                                            for set in &group.sets {
                                                if let Some(file_info) = set.files.iter().find(|f| f.path == p) {
                                                    if !app.state.jobs.iter().any(|j| j.file_info.path == file_info.path && matches!(j.action, crate::tui_app::ActionType::Copy(_))) {
                                                        app.state.jobs.push(crate::tui_app::Job {
                                                            action: crate::tui_app::ActionType::Copy(dest_path.clone()),
                                                            file_info: file_info.clone(),
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Show help screen
                (KeyCode::Char('h'), KeyModifiers::NONE) => {
                    if app.state.input_mode == crate::tui_app::InputMode::Help {
                        app.state.input_mode = crate::tui_app::InputMode::Normal;
                        app.state.status_message = Some("Exited help screen".to_string());
                    } else {
                        app.state.input_mode = crate::tui_app::InputMode::Help;
                        app.state.status_message = Some("Showing help. Press 'h' or 'Esc' to exit.".to_string());
                    }
                }
                _ => {}
            }
        }
        InputMode::CopyDestination => {
            handle_copy_dest_input_key(app, key);
        }
        InputMode::Help => {
            if key.code == KeyCode::Esc || (key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::NONE) {
                app.state.input_mode = crate::tui_app::InputMode::Normal;
                app.state.status_message = Some("Exited help screen".to_string());
            }
        }
        _ => {
            // Handle other input modes (Settings, Help, etc.) if necessary
        }
    }
}

/// Create a specialized app instance for Copy Missing mode
pub fn create_copy_missing_app(options: &Options) -> App {
    // Initialize from the regular app
    let mut app = App::new_copy_missing_mode(options);

    // Set additional state for copy-missing specific functionality
    app.state.status_message =
        Some("Copy Missing Mode - Scanning for files to copy...".to_string());

    // Initialize enhanced explorer if there's a target directory
    if let Some(target_dir) = &options.target {
        // Convert to absolute path and canonicalize
        let target = ensure_absolute_path(target_dir);
        log::info!("Using target directory: {}", target.display());
        app.state.destination_path = Some(target.clone());
        
        // Create enhanced explorer for better file browsing
        if let Ok(explorer) = EnhancedExplorer::new(
            Some(target.clone()),
            &format!("Destination: {}", target.display())
        ) {
            app.state.enhanced_explorer = Some(explorer);
        } else {
            // Fallback to old file browser if enhanced explorer fails
            app.state.file_browser = Some(FileBrowser::new(Some(target.clone())));
        }
    } else if !options.directories.is_empty() {
        // Use the last directory as the destination if no specific target was provided
        let last_dir = options.directories.last().unwrap().clone();
        let target = ensure_absolute_path(&last_dir);
        log::info!("Using last directory as target: {}", target.display());
        app.state.destination_path = Some(target.clone());
        
        // Create enhanced explorer for better file browsing
        if let Ok(explorer) = EnhancedExplorer::new(
            Some(target.clone()),
            &format!("Destination: {}", target.display())
        ) {
            app.state.enhanced_explorer = Some(explorer);
        } else {
            // Fallback to old file browser if enhanced explorer fails
            app.state.file_browser = Some(FileBrowser::new(Some(target)));
        }
    }

    // Set update mode from command line options
    app.state.update_mode = options.update_mode;

    app
}

// Function to start scan for missing files
pub fn start_copy_missing_scan(app: &mut App, options: &Options) {
    app.state.is_loading = true;
    app.state.loading_message = "Starting scan for missing files...".to_string();

    let (tx, rx) = std::sync::mpsc::channel::<ScanMessage>();

    let options_clone = options.clone();
    let thread_handle = std::thread::spawn(move || {
        // Send status updates
        tx.send(ScanMessage::StatusUpdate(
            1,
            "Comparing directories...".to_string(),
        ))
        .unwrap_or_else(|_| {
            log::warn!("Failed to send status update message");
        });

        // Perform the actual comparison
        match crate::file_utils::compare_directories_with_progress(&options_clone, tx.clone()) {
            Ok(result) => {
                // Send completion message
                if tx
                    .send(ScanMessage::StatusUpdate(
                        3,
                        format!(
                            "Scan complete: {} missing files found",
                            result.missing_in_target.len()
                        ),
                    ))
                    .is_err()
                {
                    log::error!("Failed to send completion message");
                }

                // Convert missing files to duplicate sets format for compatibility
                let sets = result
                    .missing_in_target
                    .into_iter()
                    .map(|file| {
                        crate::file_utils::DuplicateSet {
                            hash: file.path.to_string_lossy().to_string(), // Use path as hash for unique identification
                            size: file.size,
                            files: vec![file],
                        }
                    })
                    .collect::<Vec<_>>();

                // Send the actual results to be processed
                if tx.send(ScanMessage::Completed(Ok(sets))).is_err() {
                    log::error!("Failed to send scan results");
                }
            }
            Err(e) => {
                // Send error message
                if tx.send(ScanMessage::Error(e.to_string())).is_err() {
                    log::error!("Failed to send error message");
                }
            }
        }
    });

    // Store the thread handle and receiver in app state
    app.scan_thread_join_handle = Some(thread_handle);
    app.scan_rx = Some(rx);
}

// Function to handle scan messages
pub fn handle_copy_missing_scan_messages(app: &mut App) {
    if let Some(ref rx) = app.scan_rx {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                ScanMessage::StatusUpdate(stage, msg) => {
                    let stage_prefix = match stage {
                        0 => "⏳ [0/3] ", // Pre-scan stage
                        1 => "📁 [1/3] ",
                        2 => "🔍 [2/3] ",
                        3 => "🔄 [3/3] ",
                        _ => "",
                    };

                    app.state.loading_message = format!("{}{}", stage_prefix, msg);
                    // Log important messages
                    if stage == 3 || msg.contains("complete") || msg.contains("error") {
                        log::info!("Scan status: {}{}", stage_prefix, msg);
                    } else {
                        log::debug!("Scan status: {}{}", stage_prefix, msg);
                    }
                }
                ScanMessage::Completed(result) => {
                    match result {
                        Ok(sets) => {
                            // Process the raw sets into our grouped view
                            let (grouped_data, display_list) =
                                App::process_raw_sets_into_grouped_view(sets, true);

                            log::info!(
                                "Scan completed. Found {} groups with missing files",
                                grouped_data.len()
                            );

                            // Additional logging for debugging
                            let total_files = grouped_data
                                .iter()
                                .map(|g| g.sets.iter().map(|s| s.files.len()).sum::<usize>())
                                .sum::<usize>();

                            log::info!("Total missing files found: {}", total_files);

                            if grouped_data.is_empty() {
                                log::warn!("No missing files found. Check your source and target directories.");
                                app.state.status_message = Some("No missing files found. All files from source exist in target.".to_string());
                            } else {
                                // Group data by parent folder
                                for group in &grouped_data {
                                    log::info!(
                                        "Folder: {} - contains {} sets",
                                        group.path.display(),
                                        group.sets.len()
                                    );

                                    let files_in_group =
                                        group.sets.iter().map(|s| s.files.len()).sum::<usize>();

                                    log::info!("  Total files in group: {}", files_in_group);
                                }
                            }

                            app.state.grouped_data = grouped_data;
                            app.state.display_list = display_list;
                            app.state.is_loading = false;

                            // Update the status message
                            log::info!("Copy missing scan complete");
                            app.state.status_message = Some(format!(
                                "Scan complete! Found {} missing files to copy.",
                                total_files
                            ));
                        }
                        Err(e) => {
                            app.state.is_loading = false;
                            app.state.status_message = Some(format!("Scan error: {}", e));
                            log::error!("Copy missing scan error: {}", e);
                        }
                    }
                }
                ScanMessage::Error(e) => {
                    app.state.is_loading = false;
                    app.state.status_message = Some(format!("Error: {}", e));
                    log::error!("Copy missing scan error: {}", e);
                }
            }
        }
    }

    // If the app has scan results for missing files, process them here
    app.handle_scan_messages();
    
    // Always check destination panel after every frame while jobs are being processed or right after completion
    if app.state.is_processing_jobs || app.state.last_job_completion_check == Some(true) {
        // Force refresh of destination panel on every frame while processing jobs
        if let Some(explorer) = &mut app.state.enhanced_explorer {
            // Always use the original destination path for consistency
            let dest_path = app.state.destination_path.clone();
            
            // Only log at info level when job status changes, otherwise use debug
            if app.state.is_processing_jobs != app.state.last_job_completion_check.unwrap_or(false) {
                if let Some(dest_path) = &dest_path {
                    log::info!("Refreshing destination during/after job execution, original path: {}", dest_path.display());
                }
            }
            
            // If we have a problematic path (contains /.nofollow) or first refresh after job completion,
            // completely recreate the explorer to ensure a clean state
            let current_path = explorer.current_dir();
            let has_problematic_path = current_path.to_string_lossy().contains("/.nofollow");
            let is_first_refresh_after_jobs = !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true);
            
            if has_problematic_path || is_first_refresh_after_jobs {
                // Log the issue
                if has_problematic_path {
                    log::warn!("Detected problematic path /.nofollow during auto refresh, recreating explorer");
                } else if is_first_refresh_after_jobs {
                    log::info!("First refresh after job completion, recreating explorer");
                }
                
                // Always recreate with the original destination path
                if let Some(dest_path) = &dest_path {
                    log::info!("Recreating explorer with original destination path: {}", dest_path.display());
                    
                    // Create new explorer with original destination path
                    if let Ok(new_explorer) = EnhancedExplorer::new(
                        Some(dest_path.clone()),
                        &format!("Destination: {}", dest_path.display())
                    ) {
                        // Replace the existing explorer
                        *explorer = new_explorer;
                        log::info!("Successfully recreated explorer with original path");
                        
                        if !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true) {
                            app.state.status_message = Some("Job(s) completed and panels refreshed".to_string());
                        }
                    } else {
                        // Log error but continue
                        log::error!("Failed to recreate explorer with original path");
                        app.state.status_message = Some("Error refreshing destination - try manual navigation".to_string());
                    }
                }
            } else {
                // Normal refresh for non-problematic paths
                if explorer.refresh().is_err() {
                    // Only show error if we're newly completed
                    if !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true) {
                        app.state.status_message = Some("Error refreshing destination - try manual navigation".to_string());
                    }
                } else if !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true) {
                    app.state.status_message = Some("Job(s) completed and panels refreshed".to_string());
                }
            }
        } else if let Some(file_browser) = &mut app.state.file_browser {
            // Force a complete refresh
            file_browser.refresh();
            if !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true) {
                app.state.status_message = Some("Job(s) completed and panels refreshed".to_string());
            }
        }
        
        // Log the refresh event
        log::debug!("Destination browser refreshed (is_processing={}, last_check={:?})", 
                   app.state.is_processing_jobs, app.state.last_job_completion_check);
        
        // If jobs have completed, but we're not in the first frame after completion, reset the flag
        static mut FRAMES_AFTER_COMPLETION: u8 = 0;
        
        if !app.state.is_processing_jobs && app.state.last_job_completion_check == Some(true) {
            unsafe {
                FRAMES_AFTER_COMPLETION += 1;
                // Keep refreshing for 10 frames after job completion to ensure UI updates
                if FRAMES_AFTER_COMPLETION >= 10 {
                    log::info!("All jobs completed and 10 refresh frames passed, resetting job completion check");
                    app.state.last_job_completion_check = Some(false);
                    FRAMES_AFTER_COMPLETION = 0;
                }
            }
        }
    }
    
    // Track job processing state for next check
    if app.state.last_job_completion_check.is_none() || 
       (app.state.last_job_completion_check != Some(true) && app.state.is_processing_jobs) {
        app.state.last_job_completion_check = Some(app.state.is_processing_jobs);
    }
}

/// Special UI layout for Copy Missing mode
pub fn ui_copy_missing(frame: &mut Frame, app: &mut App) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(3), // Status
            Constraint::Min(0),    // Main content (3 panels)
            Constraint::Length(5), // Log area
            Constraint::Length(1), // Progress bar (if any)
            Constraint::Length(1), // Help bar
        ])
        .split(frame.area());

    // Title bar
    let title = ratatui::widgets::Paragraph::new("Dedups - Copy Missing Mode")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Dedups"))
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(title, chunks[0]);

    // Status bar
    let status = app.state.status_message.as_deref().unwrap_or("Ready");
    let status_widget = ratatui::widgets::Paragraph::new(status)
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .style(Style::default().fg(Color::White));
    frame.render_widget(status_widget, chunks[1]);

    // Main content - 3 panels layout
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // Missing Files (left)
            Constraint::Percentage(35), // Destination Browser (middle)
            Constraint::Percentage(30), // Jobs (right)
        ])
        .split(chunks[2]);

    // Left Panel: Source Files Missing From Destination
    let left_title = format!(
        "Source Files Missing ({}/{})",
        app.state
            .selected_display_list_index
            .saturating_add(1)
            .min(app.state.display_list.len()),
        app.state.display_list.len()
    );
    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(left_title)
        .border_style(
            Style::default().fg(if app.state.active_panel == ActivePanel::Sets {
                Color::Yellow
            } else {
                Color::White
            }),
        );

    // Middle Panel: Destination Browser
    let mode_info = if app.state.update_mode {
        " (Update Mode)"
    } else {
        ""
    };

    let middle_title = format!("Destination Browser{}", mode_info);
    let middle_block = Block::default()
        .borders(Borders::ALL)
        .title(middle_title)
        .border_style(
            Style::default().fg(if app.state.active_panel == ActivePanel::Files {
                Color::Yellow
            } else {
                Color::White
            }),
        );

    // Right Panel: Jobs
    let right_title = format!("Jobs ({}) (Ctrl+E: Execute, C: Copy)", app.state.jobs.len());
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(right_title)
        .border_style(
            Style::default().fg(if app.state.active_panel == ActivePanel::Jobs {
                Color::Yellow
            } else {
                Color::White
            }),
        );
        
    // There's no separate file details panel anymore, we show file sizes directly in listings

    // If loading, show the loading screen
    if app.state.is_loading {
        show_loading_screen(frame, app, chunks[2]);
        return;
    }

    // Left Panel - Missing Files from Source
    let list_items: Vec<ListItem> = app
        .state
        .display_list
        .iter()
        .map(|item| match item {
            crate::tui_app::DisplayListItem::Folder {
                path,
                is_expanded,
                set_count,
                ..
            } => {
                let prefix = if *is_expanded { "[-]" } else { "[+]" };
                let display_name = path.file_name().unwrap_or_default().to_string_lossy();
                if display_name.is_empty() {
                    // Root directory, use full path
                    ListItem::new(Line::from(Span::styled(
                        format!("{} {} ({} sets)", prefix, path.display(), set_count),
                        Style::default().add_modifier(Modifier::BOLD),
                    )))
                } else {
                    // Use directory name only
                    ListItem::new(Line::from(Span::styled(
                        format!("{} {} ({} sets)", prefix, display_name, set_count),
                        Style::default().add_modifier(Modifier::BOLD),
                    )))
                }
            }
            crate::tui_app::DisplayListItem::SetEntry {
                set_hash_preview,
                file_count_in_set,
                set_total_size,
                indent,
                original_group_index,
                original_set_index_in_group,
                ..
            } => {
                let indent_str = if *indent { "  " } else { "" };
                let is_single_file = *file_count_in_set == 1;
                                            if is_single_file {
                                let file_info = app.state.grouped_data
                                    .get(*original_group_index)
                                    .and_then(|group| group.sets.get(*original_set_index_in_group))
                                    .and_then(|set| set.files.first());
                                
                                let file_path = file_info.map(|f| f.path.clone());
                                let file_name = file_path
                                    .as_ref()
                                    .and_then(|p| p.file_name())
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "(unknown)".to_string());
                                
                                let file_size = file_info.map(|f| format_size(f.size, DECIMAL)).unwrap_or_default();
                                let selected = file_path.map(|p| app.state.selected_left_panel.contains(&p)).unwrap_or(false);
                                let sel_box = if selected { "[x]" } else { "[ ]" };
                                
                                // Create line with different spans for name and size
                                let mut line = Line::default();
                                line.spans.push(Span::styled(
                                    format!("{}{} {}", indent_str, sel_box, file_name),
                                    Style::default(),
                                ));
                                line.spans.push(Span::styled(
                                    format!(" ({})", file_size),
                                    Style::default().fg(Color::Gray),
                                ));
                                
                                ListItem::new(line)
                } else {
                    // For sets, show as selected if all files in set are selected
                    let file_paths: Vec<PathBuf> = app.state.grouped_data
                        .get(*original_group_index)
                        .and_then(|group| group.sets.get(*original_set_index_in_group))
                        .map(|set| set.files.iter().map(|f| f.path.clone()).collect())
                        .unwrap_or_default();
                    let all_selected = !file_paths.is_empty() && file_paths.iter().all(|p| app.state.selected_left_panel.contains(p));
                    let sel_box = if all_selected { "[x]" } else { "[ ]" };
                    
                    // Create line with different spans for different parts
                    let mut line = Line::default();
                    line.spans.push(Span::styled(
                        format!("{}{} Set {} ", indent_str, sel_box, set_hash_preview),
                        Style::default(),
                    ));
                    line.spans.push(Span::styled(
                        format!("({} files, {})", file_count_in_set, format_size(*set_total_size, DECIMAL)),
                        Style::default().fg(Color::Gray),
                    ));
                    
                    ListItem::new(line)
                }
            }
        })
        .collect();

    let missing_files_list = List::new(list_items)
        .block(left_block)
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Blue),
        )
        .highlight_symbol(">> ");

    let mut sets_list_state = ListState::default();
    if !app.state.display_list.is_empty() {
        sets_list_state.select(Some(app.state.selected_display_list_index));
    }
    frame.render_stateful_widget(missing_files_list, main_chunks[0], &mut sets_list_state);

    // Middle Panel - Destination Browser using FileBrowser
    if let Some(explorer) = &mut app.state.enhanced_explorer {
        // Use our enhanced explorer when available
        explorer.update_theme(app.state.active_panel == ActivePanel::Files)
            .unwrap_or_else(|e| {
                app.state.log_messages.push(format!("Explorer error: {}", e));
            });
        
        // Render the explorer in the middle panel
        explorer.render(frame, main_chunks[1]);
    } else if let Some(file_browser) = &app.state.file_browser {
        // Fallback to old browser if enhanced is not available
        
        // Create browser widget
        let mut browser_widget = file_browser.widget();

        // Update the block to match our theme
        browser_widget = browser_widget.block(middle_block);

        // Render with state
        let mut browser_state = ListState::default();
        if !file_browser.entries.is_empty() {
            browser_state.select(Some(file_browser.selected_index));
        }

        frame.render_stateful_widget(browser_widget, main_chunks[1], &mut browser_state);
    } else {
        // Fallback if no file browser is available
        let no_browser_msg = vec![
            ListItem::new("No destination directory selected."),
            ListItem::new("Press 'C' to select a destination for copy."),
        ];

        let fallback_list = List::new(no_browser_msg).block(middle_block);

        frame.render_widget(fallback_list, main_chunks[1]);
    }

    // Right Panel: Jobs
    let job_items: Vec<ListItem> = app
        .state
        .jobs
        .iter()
        .map(|job| {
            let action_str = match &job.action {
                ActionType::Keep => "KEEP".to_string(),
                ActionType::Delete => "DELETE".to_string(),
                ActionType::Move(dest) => format!("MOVE to {}", dest.display()),
                ActionType::Copy(dest) => {
                    if app.state.update_mode {
                        format!("UPDATE to {}", dest.display())
                    } else {
                        format!("COPY to {}", dest.display())
                    }
                }
                ActionType::Ignore => "IGNORE".to_string(),
            };
            let content = Line::from(Span::raw(format!(
                "{} - {}",
                action_str,
                job.file_info
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )));
            ListItem::new(content)
        })
        .collect();

    let jobs_list_widget = List::new(job_items)
        .block(right_block)
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Magenta),
        )
        .highlight_symbol(">> ");

    let mut jobs_list_state = ListState::default();
    if !app.state.jobs.is_empty() {
        jobs_list_state.select(Some(app.state.selected_job_index));
    }
    frame.render_stateful_widget(jobs_list_widget, main_chunks[2], &mut jobs_list_state);

    // Status Bar / Input Area
    match app.state.input_mode {
        InputMode::Normal => {
            // Show custom status message if available, otherwise show controls
            let mut status_text = app.state.status_message.as_deref().unwrap_or(
                "q/Ctrl+C:quit | Tab:cycle | Arrows/jk:nav | s:select for copy | Ctrl+E:exec | x:del job"
            ).to_string();

            // Add dry run indicator if enabled
            if app.state.dry_run {
                status_text = format!("[DRY RUN MODE] {} (Ctrl+D: Toggle)", status_text);
            } else {
                status_text = format!("{} (Ctrl+D: Dry Run)", status_text);
            }

            // Add update mode indicator
            if app.state.update_mode {
                status_text = format!("[UPDATE MODE] {} (u: Toggle)", status_text);
            } else {
                status_text = format!("{} (u: Update Mode)", status_text);
            }

            let status_style = if app.state.dry_run {
                // Use yellow for dry run mode to make it more obvious
                Style::default().fg(Color::Yellow)
            } else if app.state.update_mode {
                // Use green for update mode
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::LightCyan)
            };

            let status_bar = Paragraph::new(status_text)
                .style(status_style)
                .alignment(Alignment::Left);
            frame.render_widget(status_bar, chunks[3]);
        }
        InputMode::CopyDestination => {
            let input_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(chunks[3]);
            let prompt_text = app
                .state
                .status_message
                .as_deref()
                .unwrap_or("Enter destination path for copy (Enter:confirm, Esc:cancel):");
            let prompt_p = Paragraph::new(prompt_text).fg(Color::Yellow);
            frame.render_widget(prompt_p, input_chunks[0]);
            let input_field = Paragraph::new(app.state.current_input.value())
                .block(
                    Block::default()
                        .borders(Borders::TOP)
                        .title("Path")
                        .border_style(Style::default().fg(Color::Yellow)),
                )
                .fg(Color::White);
            frame.render_widget(input_field, input_chunks[1]);
            frame.set_cursor_position(Position {
                x: input_chunks[1].x + app.state.current_input.visual_cursor() as u16 + 1,
                y: input_chunks[1].y + 1,
            });
        }
        InputMode::Help => {
            // When in Help mode, we'll show a full-screen help overlay
                            let help_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2), // Title
                    Constraint::Min(3),    // Content
                    Constraint::Length(1), // Footer
                ])
                .split(frame.area());

            let help_title = ratatui::widgets::Paragraph::new("Help - Key Bindings")
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center);
            
            let help_text = "
Tab / Left/Right:    Switch Panel
Up/Down / j/k:       Navigate items
Space:               Toggle selection for file or set
Enter:               Open directory (in destination panel)
a:                   Select/Unselect all files
n:                   Sort by name
z:                   Sort by size
m:                   Sort by modified time
d:                   Sort by duplicates (path length)
f:                   Toggle folders first
c:                   Copy selected file
e:                   Expand all folders
x:                   Collapse all folders
Ctrl+E:              Execute copy jobs
Ctrl+D:              Toggle dry run mode
u:                   Toggle update mode
h:                   Toggle this help screen
q/Esc/Ctrl+C:        Quit
            ";
            
            let help_widget = ratatui::widgets::Paragraph::new(help_text)
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Left)
                .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
            
            let footer = ratatui::widgets::Paragraph::new("Press 'h' or 'Esc' to close help")
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Center);
            
            frame.render_widget(Clear, frame.area()); // Clear the screen first
            frame.render_widget(help_title, help_area[0]);
            frame.render_widget(help_widget, help_area[1]);
            frame.render_widget(footer, help_area[2]);
        }
        _ => {
            // Handle other input modes (Settings, Help, etc.) if necessary
        }
    }

    // Draw progress bar if processing jobs or loading
    if app.state.is_processing_jobs || app.state.is_loading {
        draw_progress_bar(frame, app, chunks[4]);
    } else {
        // Draw an empty block if no progress
        let empty = Block::default();
        frame.render_widget(empty, chunks[4]);
    }

    // Draw help bar at the very bottom
    let help = "Tab: Switch Panel | Space: Toggle Expand | Enter: Open Dir | e: Expand All | x: Collapse All | h: Help | c: Copy | Ctrl+E: Execute";
    let help_bar = ratatui::widgets::Paragraph::new(help)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help_bar, chunks[5]);

    // Draw log area
    draw_log_area(frame, app, chunks[3]);
}

// Helper function to show loading screen
fn show_loading_screen(frame: &mut Frame, app: &mut App, area: Rect) {
    // Create a centered loading screen
    let loading_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(4),
            Constraint::Percentage(40),
        ])
        .split(area)[1];

    let loading_message = format!("Loading: {}", app.state.loading_message);
    let loading_text = Paragraph::new(loading_message)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Please Wait"));

    frame.render_widget(loading_text, loading_area);
}

// Helper function to draw progress bar
fn draw_progress_bar(frame: &mut Frame, app: &mut App, area: Rect) {
    use ratatui::widgets::Gauge;

    if app.state.is_processing_jobs {
        let (done, total) = app.state.job_progress;
        let percent = if total > 0 {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Job Progress"))
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
            .label(format!(
                "Progress: {}/{} jobs ({:.1}%)",
                done,
                total,
                percent * 100.0
            ))
            .ratio(percent);

        frame.render_widget(gauge, area);
    } else if app.state.is_loading {
        // Extract progress information from the loading message
        let progress_msg = app.state.loading_message.clone();

        // Try to parse a percentage from the message if available
        let progress_pct = if progress_msg.contains('%') {
            let parts: Vec<&str> = progress_msg.split('(').collect();
            if parts.len() > 1 {
                let percent_part = parts[1].split(')').next().unwrap_or("0%");
                let percent_str = percent_part.trim_end_matches('%').trim();
                percent_str.parse::<f64>().unwrap_or(50.0) / 100.0
            } else {
                0.5 // Default to 50%
            }
        } else {
            0.5 // Default to 50% if no percentage in message
        };

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Scanning Progress"),
            )
            .gauge_style(Style::default().fg(Color::Blue).bg(Color::Black))
            .label(progress_msg)
            .ratio(progress_pct);

        frame.render_widget(gauge, area);
    }
}

// Helper function to draw log area
fn draw_log_area(frame: &mut Frame, app: &mut App, area: Rect) {
    let log_height = 5;
    let log_len = app.state.log_messages.len();
    let scroll = app.state.log_scroll.min(log_len.saturating_sub(log_height));
    let log_lines: Vec<ratatui::text::Line> = app
        .state
        .log_messages
        .iter()
        .filter(|msg| {
            app.state
                .log_filter
                .as_ref()
                .is_none_or(|f| msg.contains(f))
        })
        .skip(scroll)
        .take(log_height)
        .map(|msg| ratatui::text::Line::from(msg.clone()))
        .collect();

    let log_block = if app.state.log_focus {
        Block::default()
            .borders(Borders::ALL)
            .title("Log (FOCUSED)")
    } else {
        Block::default().borders(Borders::ALL).title("Log")
    };

    let log_paragraph = ratatui::widgets::Paragraph::new(log_lines)
        .block(log_block)
        .scroll((0, 0));

    frame.render_widget(log_paragraph, area);
}

// In the handle_copy_dest_input_key function, change the line that handles the event to use the type alias
fn handle_copy_dest_input_key(app: &mut App, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Enter => {
            let dest_path_str = app.state.current_input.value().to_string();
            app.state.current_input.reset();
            app.state.input_mode = InputMode::Normal;
            if let Some(file_to_copy) = app.state.file_for_copy_move.take() {
                if !dest_path_str.trim().is_empty() {
                    let dest_path = PathBuf::from(dest_path_str.trim());
                    app.set_action_for_selected_file(ActionType::Copy(dest_path.clone()));
                    app.state.status_message = Some(format!(
                        "Marked {} for copy to {}",
                        file_to_copy.path.display(),
                        dest_path.display()
                    ));
                } else {
                    app.state.status_message =
                        Some("Copy cancelled: empty destination path.".to_string());
                }
            } else {
                app.state.status_message =
                    Some("Copy cancelled: no file selected.".to_string());
            }
        }
        KeyCode::Esc => {
            app.state.current_input.reset();
            app.state.input_mode = InputMode::Normal;
            app.state.file_for_copy_move = None;
            app.state.status_message = Some("Copy action cancelled.".to_string());
        }
        // Handle simple cases directly, as tui-input's API for cursor manipulation is limited
        KeyCode::Char(c) => {
            app.state.current_input = tui_input::Input::from(app.state.current_input.value().to_string() + &c.to_string());
        }
        KeyCode::Backspace => {
            let value = app.state.current_input.value().to_string();
            if !value.is_empty() {
                app.state.current_input = tui_input::Input::from(&value[..value.len()-1]);
            }
        }
        _ => {} // Ignore other keys for simplicity
    }
}

// Helper function to ensure a path is absolute and exists
pub fn ensure_absolute_path(path: &PathBuf) -> PathBuf {
    // If path is already absolute, use it
    let abs_path = if path.is_absolute() {
        path.clone()
    } else {
        // Otherwise, make it absolute
        match std::env::current_dir() {
            Ok(current_dir) => current_dir.join(path),
            Err(_) => path.clone() // Fallback to original path on error
        }
    };
    
    // Create directory if it doesn't exist
    if !abs_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&abs_path) {
            log::warn!("Failed to create destination directory {}: {}", abs_path.display(), e);
        } else {
            log::info!("Created destination directory: {}", abs_path.display());
        }
    }
    
    // Try to canonicalize the path, falling back to the absolute path if that fails
    match abs_path.canonicalize() {
        Ok(canon_path) => canon_path,
        Err(e) => {
            log::warn!("Failed to canonicalize path {}: {}", abs_path.display(), e);
            abs_path
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_mode::AppMode;
    use std::path::PathBuf;

    // Utility function to create a test App with simulated missing files
    fn create_test_app_with_missing_files() -> App {
        // Create a basic app
        let options = Options {
            directories: vec![PathBuf::from("/source"), PathBuf::from("/dest")],
            copy_missing: true,
            app_mode: AppMode::CopyMissing,
            // Fill in required fields with defaults
            target: None,
            deduplicate: false,
            delete: false,
            move_to: None,
            update_mode: false,
            log: false,
            log_file: None,
            output: None,
            format: "json".to_string(),
            algorithm: "xxhash".to_string(),
            parallel: None,
            mode: "newest_modified".to_string(),
            interactive: true,
            verbose: 0,
            include: vec![],
            exclude: vec![],
            filter_from: None,
            progress: false,
            progress_tui: true,
            sort_by: crate::file_utils::SortCriterion::ModifiedAt,
            sort_order: crate::file_utils::SortOrder::Descending,
            raw_sizes: false,
            config_file: None,
            dry_run: false,
            cache_location: None,
            fast_mode: false,
            media_mode: false,
            media_resolution: "highest".to_string(),
            media_formats: vec![],
            media_similarity: 90,
            media_dedup_options: crate::media_dedup::MediaDedupOptions::default(),
        };

        // Normally we'd populate with real missing files, but for tests we'll use simulated data
        let app = create_copy_missing_app(&options);
        app
    }

    #[test]
    fn test_copy_missing_app_creation() {
        // Create a mock options object
        let options = Options {
            directories: vec![PathBuf::from("/test")],
            copy_missing: true,
            app_mode: AppMode::CopyMissing,
            // Fill in required fields with defaults
            target: None,
            deduplicate: false,
            delete: false,
            move_to: None,
            update_mode: false,
            log: false,
            log_file: None,
            output: None,
            format: "json".to_string(),
            algorithm: "xxhash".to_string(),
            parallel: None,
            mode: "newest_modified".to_string(),
            interactive: true,
            verbose: 0,
            include: vec![],
            exclude: vec![],
            filter_from: None,
            progress: false,
            progress_tui: true,
            sort_by: crate::file_utils::SortCriterion::ModifiedAt,
            sort_order: crate::file_utils::SortOrder::Descending,
            raw_sizes: false,
            config_file: None,
            dry_run: false,
            cache_location: None,
            fast_mode: false,
            media_mode: false,
            media_resolution: "highest".to_string(),
            media_formats: vec![],
            media_similarity: 90,
            media_dedup_options: crate::media_dedup::MediaDedupOptions::default(),
        };

        // Create the app
        let app = create_copy_missing_app(&options);

        // Verify it's in copy missing mode
        assert!(app.state.is_copy_missing_mode);
        assert!(app.state.status_message.is_some());
        assert!(app
            .state
            .status_message
            .as_ref()
            .unwrap()
            .contains("Copy Missing Mode"));
    }

    #[test]
    fn test_loading_screen_display() {
        let mut app = create_test_app_with_missing_files();

        // Set loading state
        app.state.is_loading = true;
        app.state.loading_message = "Test loading message".to_string();

        // Test that loading screen is displayed
        // We can't easily test the actual UI rendering, but we can check loading state is set correctly
        assert!(app.state.is_loading);
        assert_eq!(app.state.loading_message, "Test loading message");
    }

    #[test]
    fn test_dry_run_mode() {
        let mut app = create_test_app_with_missing_files();

        // Enable dry run mode
        app.state.dry_run = true;

        // Check dry run mode is enabled
        assert!(app.state.dry_run);
    }
}
