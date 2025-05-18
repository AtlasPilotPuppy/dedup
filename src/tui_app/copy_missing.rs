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
use std::collections::HashMap;
use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::file_utils::{self, DuplicateSet, FileInfo, SelectionStrategy};
use crate::options::Options;
use crate::tui_app::{ActivePanel, ActionType, App, AppState, InputMode, Job, ScanMessage};

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

    // Handle initial scan messages
    app.handle_scan_messages();

    loop {
        // Continue to handle messages
        app.handle_scan_messages();

        terminal.draw(|f| ui_copy_missing(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let CEvent::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }

        // Reset the tick timer even if no event was processed
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Cleanup and restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}

/// Create a specialized app instance for Copy Missing mode
pub fn create_copy_missing_app(options: &Options) -> App {
    // Initialize from the regular app
    let mut app = App::new(options);
    
    // Modify for Copy Missing mode
    app.state.is_copy_missing_mode = true;
    app.state.status_message = Some("Copy Missing Mode - Looking for files to copy...".to_string());
    
    app
}

/// Special UI layout for Copy Missing mode
fn ui_copy_missing(frame: &mut Frame, app: &mut App) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(3), // Status
            Constraint::Min(0),    // Main content
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
            Constraint::Percentage(35), // Destination Files (middle)
            Constraint::Percentage(30), // Jobs (right)
        ])
        .split(chunks[2]);

    // Left Panel: Missing Files from Source
    let left_title = format!(
        "Source Files Missing From Destination ({}/{})",
        app.state.selected_display_list_index.saturating_add(1).min(app.state.display_list.len()),
        app.state.display_list.len()
    );
    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(left_title)
        .border_style(Style::default().fg(
            if app.state.active_panel == ActivePanel::Sets { 
                Color::Yellow 
            } else { 
                Color::White 
            }));

    // Middle Panel: Destination Files
    let middle_title = "Destination Files (Browse)";
    let middle_block = Block::default()
        .borders(Borders::ALL)
        .title(middle_title)
        .border_style(Style::default().fg(
            if app.state.active_panel == ActivePanel::Files { 
                Color::Yellow 
            } else { 
                Color::White 
            }));

    // Right Panel: Jobs
    let right_title = format!("Jobs ({}) (Ctrl+E: Execute, x: Remove)", app.state.jobs.len());
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(right_title)
        .border_style(Style::default().fg(
            if app.state.active_panel == ActivePanel::Jobs { 
                Color::Yellow 
            } else { 
                Color::White 
            }));

    // If loading, show the loading screen
    if app.state.is_loading {
        show_loading_screen(frame, app, chunks[2]);
        return;
    }

    // Left Panel - Missing Files
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
                ListItem::new(Line::from(Span::styled(
                    format!("{} {} ({} sets)", prefix, path.display(), set_count),
                    Style::default().add_modifier(Modifier::BOLD),
                )))
            }
            crate::tui_app::DisplayListItem::SetEntry {
                set_hash_preview,
                set_total_size,
                file_count_in_set,
                indent,
                ..
            } => {
                let indent_str = if *indent { "  " } else { "" };
                ListItem::new(Line::from(Span::styled(
                    format!(
                        "{}Missing: {} files, {}",
                        indent_str,
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

    // Middle Panel - Currently Selected Set Files
    let (files_title, file_items) = if let Some(selected_set) =
        app.current_selected_set_from_display_list()
    {
        let title = format!(
            "Files ({}/{}) (c:copy f:filter)",
            app.state
                .selected_file_index_in_set
                .saturating_add(1)
                .min(selected_set.files.len()),
            selected_set.files.len()
        );
        let items: Vec<ListItem> = selected_set
            .files
            .iter()
            .map(|file_info| {
                let mut style = Style::default();
                let mut prefix = "   ";
                if let Some(job) = app
                    .state
                    .jobs
                    .iter()
                    .find(|j| j.file_info.path == file_info.path)
                {
                    match job.action {
                        ActionType::Keep => {
                            style = style.fg(Color::Green).add_modifier(Modifier::BOLD);
                            prefix = "[K]";
                        }
                        ActionType::Delete => {
                            style = style.fg(Color::Red).add_modifier(Modifier::CROSSED_OUT);
                            prefix = "[D]";
                        }
                        ActionType::Copy(_) => {
                            style = style.fg(Color::Cyan);
                            prefix = "[C]";
                        }
                        ActionType::Move(_) => {
                            style = style.fg(Color::Magenta);
                            prefix = "[M]";
                        }
                        ActionType::Ignore => {
                            style = style.fg(Color::DarkGray);
                            prefix = "[I]";
                        }
                    }
                }
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", prefix), style),
                    Span::styled(file_info.path.display().to_string(), style),
                ]))
            })
            .collect();
        (title, items)
    } else {
        (
            "Files (0/0)".to_string(),
            vec![ListItem::new("No files selected or set is empty")],
        )
    };

    let files_list = List::new(file_items)
        .block(Block::default().borders(Borders::ALL).title(files_title).border_style(
            if app.state.active_panel == ActivePanel::Files {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            },
        ))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::DarkGray),
        )
        .highlight_symbol("> ");

    let mut files_list_state = ListState::default();
    if app
        .current_selected_set_from_display_list()
        .is_some_and(|s| !s.files.is_empty())
    {
        files_list_state.select(Some(app.state.selected_file_index_in_set));
    }
    frame.render_stateful_widget(files_list, main_chunks[1], &mut files_list_state);

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
                "{} - {:?}",
                action_str,
                job.file_info.path.file_name().unwrap_or_default()
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
                "q/Ctrl+C:quit | Tab:cycle | Arrows/jk:nav | c:copy | Ctrl+E:exec | Ctrl+R:rescan | x:del job"
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
    let help = "h: Help | ↑/↓: Navigate | Space: Toggle | a: Select All | q/Ctrl+C: Quit";
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
    use crate::file_utils::{FileInfo, DuplicateSet};
    use std::path::PathBuf;

    // Utility function to create a test App with simulated missing files
    fn create_test_app_with_missing_files() -> App {
        // Create a basic app 
        let mut options = Options {
            directories: vec![PathBuf::from("/source"), PathBuf::from("/dest")],
            copy_missing: true,
            app_mode: crate::app_mode::AppMode::CopyMissing,
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

        let mut app = create_copy_missing_app(&options);
        
        // Add some simulated missing files
        let missing_files = vec![
            FileInfo {
                path: PathBuf::from("/source/file1.txt"),
                size: 1000,
                modified_at: chrono::Utc::now().timestamp(),
                created_at: chrono::Utc::now().timestamp(),
                hash: Some("file1hash".to_string()),
            },
            FileInfo {
                path: PathBuf::from("/source/file2.txt"),
                size: 2000,
                modified_at: chrono::Utc::now().timestamp(),
                created_at: chrono::Utc::now().timestamp(),
                hash: Some("file2hash".to_string()),
            },
            FileInfo {
                path: PathBuf::from("/source/subfolder/file3.txt"),
                size: 3000,
                modified_at: chrono::Utc::now().timestamp(),
                created_at: chrono::Utc::now().timestamp(),
                hash: Some("file3hash".to_string()),
            },
        ];

        // Group files by parent directory for the display
        let mut files_by_parent = std::collections::HashMap::new();
        for file in &missing_files {
            let parent = file.path.parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .to_path_buf();
            
            files_by_parent.entry(parent)
                .or_insert_with(Vec::new)
                .push(file.clone());
        }
        
        // Create DuplicateSets for each parent directory
        let mut duplicate_sets = Vec::new();
        for (parent, files) in files_by_parent {
            let total_size: u64 = files.iter().map(|f| f.size).sum();
            let set = DuplicateSet {
                hash: format!("missing_files_{}", parent.display()),
                size: total_size,
                files,
            };
            duplicate_sets.push(set);
        }

        // Process the sets into grouped view
        let (grouped_data, display_list) = 
            App::process_raw_sets_into_grouped_view(duplicate_sets, true);
        
        app.state.is_loading = false;
        app.state.grouped_data = grouped_data;
        app.state.display_list = display_list;
        
        app
    }

    #[test]
    fn test_copy_missing_app_creation() {
        // Create a mock options object
        let options = Options {
            directories: vec![PathBuf::from("/test")],
            copy_missing: true,
            app_mode: crate::app_mode::AppMode::CopyMissing,
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
        assert!(app.state.status_message.as_ref().unwrap().contains("Copy Missing Mode"));
    }

    #[test]
    fn test_copy_job_creation() {
        let mut app = create_test_app_with_missing_files();
        
        // Ensure we have at least one set with files
        assert!(!app.state.grouped_data.is_empty());
        if let Some(first_group) = app.state.grouped_data.first() {
            assert!(!first_group.sets.is_empty());
            if let Some(first_set) = first_group.sets.first() {
                assert!(!first_set.files.is_empty());
                
                // Select the first file
                app.state.selected_display_list_index = 1; // Usually the first set after folder
                app.state.selected_file_index_in_set = 0;
                
                // Get the file info
                let selected_file = app.current_selected_file().cloned();
                assert!(selected_file.is_some());
                
                if let Some(file_info) = selected_file {
                    // Create a copy job
                    let target_path = PathBuf::from("/dest");
                    let job = Job {
                        action: ActionType::Copy(target_path.clone()),
                        file_info: file_info.clone(),
                    };
                    
                    app.state.jobs.push(job);
                    
                    // Verify job was added
                    assert_eq!(app.state.jobs.len(), 1);
                    assert!(matches!(app.state.jobs[0].action, ActionType::Copy(_)));
                }
            }
        }
    }

    #[test]
    fn test_copy_missing_mode_handles_dry_run() {
        let mut app = create_test_app_with_missing_files();
        
        // Enable dry run mode
        app.state.dry_run = true;
        
        // Create a copy job
        if let Some(first_file) = app.state.grouped_data.first()
            .and_then(|group| group.sets.first())
            .and_then(|set| set.files.first()) 
        {
            app.state.jobs.push(Job {
                action: ActionType::Copy(PathBuf::from("/dest")),
                file_info: first_file.clone(),
            });
        }
        
        // Simulate processing jobs
        if !app.state.jobs.is_empty() {
            app.state.is_processing_jobs = true;
            app.state.job_progress = (0, app.state.jobs.len());
            
            // Check if processing happens in dry run mode
            assert!(app.state.dry_run);
            assert!(app.state.is_processing_jobs);
            
            // The actual job processing would happen here in the app
            // but we're just testing that dry run mode is properly set
            
            // Cleanup
            app.state.is_processing_jobs = false;
            app.state.job_progress = (0, 0);
        }
    }
} 