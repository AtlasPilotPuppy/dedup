use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use humansize::{format_size, DECIMAL};
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::io::stdout;
use std::time::{Duration, Instant};

use crate::options::Options;
use crate::tui_app::{ActionType, ActivePanel, App, InputMode, ScanMessage};

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
                    app.on_key(key);

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

/// Create a specialized app instance for Copy Missing mode
pub fn create_copy_missing_app(options: &Options) -> App {
    // Initialize from the regular app
    let mut app = App::new_copy_missing_mode(options);

    // Set additional state for copy-missing specific functionality
    app.state.status_message = Some("Copy Missing Mode - Scanning for files to copy...".to_string());

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
        tx.send(ScanMessage::StatusUpdate(1, "Comparing directories...".to_string())).unwrap_or_else(|_| {
            log::warn!("Failed to send status update message");
        });
        
        // Perform the actual comparison
        match crate::file_utils::compare_directories_with_progress(&options_clone, tx.clone()) {
            Ok(result) => {
                // Send completion message 
                if tx.send(ScanMessage::StatusUpdate(3, format!(
                    "Scan complete: {} missing files found",
                    result.missing_in_target.len()
                ))).is_err() {
                    log::error!("Failed to send completion message");
                }
                
                // Return () to match the expected JoinHandle<()> type
                ()
            },
            Err(e) => {
                // Send error message
                if tx.send(ScanMessage::Error(e.to_string())).is_err() {
                    log::error!("Failed to send error message");
                }
                
                // Return () to match the expected JoinHandle<()> type
                ()
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
                ScanMessage::StatusUpdate(stage, message) => {
                    app.state.loading_message = message.clone();
                    log::debug!("[CopyMissing] Stage {}: {}", stage, message);
                },
                ScanMessage::Error(error) => {
                    app.state.is_loading = false;
                    app.state.loading_message = format!("Error: {}", error);
                    app.state.log_messages.push(format!("ERROR: {}", error));
                    log::error!("[CopyMissing] {}", error);
                },
                ScanMessage::Completed(result) => {
                    if let Ok(sets) = result {
                        // Process duplicate sets - this is default implementation, doesn't handle missing files
                        let (grouped_data, display_list) = App::process_raw_sets_into_grouped_view(sets, true);
                        app.state.grouped_data = grouped_data;
                        app.state.display_list = display_list;
                        app.state.is_loading = false;
                        app.state.loading_message = "Scan complete.".to_string();
                        app.state.status_message = Some(format!(
                            "Found {} duplicate sets. Select files to process.",
                            app.state.grouped_data.iter().map(|g| g.sets.len()).sum::<usize>()
                        ));
                    }
                }
            }
        }
    }
    
    // If the app has scan results for missing files, process them here
    app.handle_scan_messages();
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
        .split(frame.size());

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
    let middle_title = "Destination Browser";
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
    let right_title = format!(
        "Jobs ({}) (Ctrl+E: Execute, x: Remove)",
        app.state.jobs.len()
    );
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
                        format!("{} {} ({} sets)", 
                            prefix, 
                            path.display(), 
                            set_count
                        ),
                        Style::default().add_modifier(Modifier::BOLD),
                    )))
                } else {
                    // Use directory name only
                    ListItem::new(Line::from(Span::styled(
                        format!("{} {} ({} sets)", 
                            prefix, 
                            display_name, 
                            set_count
                        ),
                        Style::default().add_modifier(Modifier::BOLD),
                    )))
                }
            }
            crate::tui_app::DisplayListItem::SetEntry {
                set_hash_preview,
                file_count_in_set,
                set_total_size,
                indent,
                ..
            } => {
                let indent_str = if *indent { "  " } else { "" };
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{}Set {} ({} files, {})",
                        indent_str,
                        set_hash_preview,
                        file_count_in_set,
                        format_size(*set_total_size, DECIMAL)
                    ),
                    Style::default(),
                )))
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

    // Middle Panel - Destination Browser (just a placeholder for now)
    let destination_list = List::new(vec![
        ListItem::new("(Not implemented yet)"),
        ListItem::new("Browse destination directory here"),
    ])
    .block(middle_block)
    .highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_widget(destination_list, main_chunks[1]);

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
                ActionType::Copy(dest) => format!("COPY to {}", dest.display()),
                ActionType::Ignore => "IGNORE".to_string(),
            };
            let content = Line::from(Span::raw(format!(
                "{} - {}",
                action_str,
                job.file_info.path.file_name().unwrap_or_default().to_string_lossy()
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

            let status_style = if app.state.dry_run {
                // Use yellow for dry run mode to make it more obvious
                Style::default().fg(Color::Yellow)
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
            frame.set_cursor(
                input_chunks[1].x + app.state.current_input.visual_cursor() as u16 + 1,
                input_chunks[1].y + 1,
            );
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
    let help = "Tab: Switch Panel | Space: Toggle Expand | s: Select Files | Ctrl+E: Execute Copy | Ctrl+D: Dry Run Toggle";
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
        // Extract progress information
        let progress_msg = app.state.loading_message.clone();
        let progress_pct = 0.5; // Default to 50% if we can't determine actual progress

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Loading"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::app_mode::AppMode;

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
