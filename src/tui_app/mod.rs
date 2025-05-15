use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyEvent, KeyModifiers, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::io::{stdout, Stdout};
use std::time::{Duration, Instant};
use std::sync::mpsc as std_mpsc; // Alias to avoid conflict if crate::mpsc is used elsewhere
use std::thread as std_thread; // Alias for clarity

use crate::Cli;
use crate::file_utils::{self, DuplicateSet, FileInfo, SelectionStrategy, delete_files, move_files}; // Added delete_files, move_files
use std::path::PathBuf; // For Job destination
use humansize::{format_size, DECIMAL}; // For displaying human-readable sizes
use tui_input::backend::crossterm::EventHandler; // For tui-input
use tui_input::Input;

// Application state
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] // Added PartialEq, Eq
pub enum ActionType {
    Keep, // Implicit action for the one file not chosen for delete/move
    Delete,
    Move(PathBuf), // Target directory for move
    Copy(PathBuf), // Target directory for copy
    Ignore, // New action type
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Job {
    pub action: ActionType,
    pub file_info: FileInfo,
    // No explicit destination here, it's part of ActionType::Move/Copy
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    Sets,
    Files,
    Jobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    CopyDestination,
    // Could add MoveDestination later if needed
}

#[derive(Debug)]
pub struct AppState {
    pub duplicate_sets: Vec<DuplicateSet>,
    pub selected_set_index: usize, // Using usize, ensure bounds checking
    pub selected_file_index_in_set: usize, // Using usize, ensure bounds checking
    pub selected_job_index: usize, // For navigating jobs
    pub jobs: Vec<Job>,
    pub active_panel: ActivePanel,
    pub default_selection_strategy: SelectionStrategy, // Store parsed strategy
    pub status_message: Option<String>, // For feedback
    pub input_mode: InputMode,
    pub current_input: Input, // Using tui-input crate
    pub file_for_copy_move: Option<FileInfo>, // Store file when prompting for dest

    // Fields for TUI loading progress
    pub is_loading: bool,
    pub loading_message: String, 
    // pub loading_progress_percent: Option<f32>, // For a gauge, if we can get good percentages
}

// Channel for messages from scan thread to TUI thread
#[derive(Debug)]
pub enum ScanMessage {
    StatusUpdate(String),
    // ProgressUpdate(f32), // If we have percentage
    Completed(Result<Vec<DuplicateSet>>),
    Error(String),
}

pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    scan_thread_join_handle: Option<std_thread::JoinHandle<()>>,
    scan_rx: Option<std_mpsc::Receiver<ScanMessage>>,
    // cli_args: Cli, // Store if needed for re-scans or passing to thread
}

impl App {
    pub fn new(cli_args: &Cli) -> Self {
        let strategy = SelectionStrategy::from_str(&cli_args.mode).unwrap_or(SelectionStrategy::NewestModified);
        let initial_status = if cli_args.progress {
            "Initializing scan...".to_string()
        } else {
            "Loading...".to_string() // Generic if progress not explicitly requested for TUI view
        };

        let mut app_state = AppState {
            duplicate_sets: Vec::new(), // Start empty, load async
            selected_set_index: 0,
            selected_file_index_in_set: 0,
            selected_job_index: 0,
            jobs: Vec::new(),
            active_panel: ActivePanel::Sets,
            default_selection_strategy: strategy,
            status_message: None,
            input_mode: InputMode::Normal,
            current_input: Input::default(),
            file_for_copy_move: None,
            is_loading: if cli_args.progress { true } else { false }, // Control initial loading state display
            loading_message: initial_status,
        };

        let (tx, rx) = std_mpsc::channel::<ScanMessage>();
        let scan_join_handle: Option<std_thread::JoinHandle<()>> = if cli_args.progress {
            let cli_clone = cli_args.clone(); // Clone cli_args for the thread
            let handle = std_thread::spawn(move || {
                // This is the background thread
                log::info!("[ScanThread] Starting duplicate scan...");
                let result = file_utils::find_duplicate_files_with_progress(&cli_clone, tx.clone());
                if tx.send(ScanMessage::Completed(result)).is_err() {
                    log::error!("[ScanThread] Failed to send completion message to TUI.");
                }
                log::info!("[ScanThread] Scan finished.");
            });
            Some(handle)
        } else {
            // Synchronous scan if --progress is not set
            log::info!("Performing synchronous scan as --progress is not set for TUI.");
            match file_utils::find_duplicate_files(cli_args) {
                Ok(sets) => {
                    app_state.duplicate_sets = sets;
                    if app_state.duplicate_sets.is_empty() {
                        app_state.status_message = Some("No duplicate sets found.".to_string());
                    }
                }
                Err(e) => {
                    log::error!("Failed to find duplicates synchronously: {}", e);
                    app_state.status_message = Some(format!("Error loading files: {}", e));
                }
            }
            None // No join handle for synchronous scan
        };
        
        if app_state.duplicate_sets.is_empty() && !cli_args.progress {
            // If sync scan and empty, selected_set_index is fine at 0.
            // Already handled: app_state.status_message = Some("No duplicate sets found.".to_string());
        } else if app_state.selected_set_index >= app_state.duplicate_sets.len() && !cli_args.progress {
             app_state.selected_set_index = app_state.duplicate_sets.len().saturating_sub(1);
        }

        Self {
            state: app_state,
            should_quit: false,
            scan_thread_join_handle: scan_join_handle,
            scan_rx: if cli_args.progress { Some(rx) } else { None },
            // cli_args: cli_args.clone(), // If needed later
        }
    }

    // Method to handle messages from the scan thread
    pub fn handle_scan_messages(&mut self) {
        if let Some(rx) = &self.scan_rx {
            while let Ok(message) = rx.try_recv() { // Use try_recv for non-blocking check
                match message {
                    ScanMessage::StatusUpdate(status) => {
                        log::debug!("TUI received scan status: {}", status);
                        self.state.loading_message = status;
                    }
                    ScanMessage::Completed(result) => {
                        log::info!("TUI received scan completion.");
                        self.state.is_loading = false;
                        match result {
                            Ok(sets) => {
                                self.state.duplicate_sets = sets;
                                if self.state.duplicate_sets.is_empty() {
                                    self.state.status_message = Some("Scan complete. No duplicate sets found.".to_string());
                                } else {
                                     self.state.status_message = Some(format!("Scan complete. Found {} sets.", self.state.duplicate_sets.len()));
                                }
                            }
                            Err(e) => {
                                self.state.loading_message = format!("Error during scan: {}", e);
                                self.state.status_message = Some(format!("Scan failed: Check logs."));
                                log::error!("Scan thread reported error: {}", e);
                            }
                        }
                        self.validate_selection_indices(); // Important after loading data
                        // Once completed, we might not need the channel anymore
                        // self.scan_rx = None; // Or keep it if re-scan is possible
                        // self.scan_thread_join_handle.take().map(|h| h.join()); // Optionally join
                        break; // Process one completion, then redraw
                    }
                    ScanMessage::Error(err_msg) => {
                        self.state.is_loading = false; // Stop loading on error
                        self.state.loading_message = format!("Scan Error: {}", err_msg);
                        self.state.status_message = Some(format!("Scan failed: {}", err_msg));
                        log::error!("Scan thread reported an error message: {}", err_msg);
                         break;
                    }
                }
            }
        }
    }

    pub fn on_key(&mut self, key_event: KeyEvent) {
        self.state.status_message = None; // Clear old status on new key press
        let key_code = key_event.code;
        let modifiers = key_event.modifiers;

        if key_code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }

        if key_code == KeyCode::Tab {
            self.cycle_active_panel();
            return;
        }

        // Ctrl+E to Execute jobs
        if key_code == KeyCode::Char('e') && modifiers == KeyModifiers::CONTROL {
            self.process_pending_jobs();
            return;
        }

        match self.state.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key_event),
            InputMode::CopyDestination => self.handle_copy_dest_input_key(key_event),
        }
        self.validate_selection_indices();
    }

    fn handle_normal_mode_key(&mut self, key_event: KeyEvent) {
        let key_code = key_event.code;
        let modifiers = key_event.modifiers;

        if key_code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }
        if key_code == KeyCode::Tab {
            self.cycle_active_panel();
            return;
        }
        if key_code == KeyCode::Char('e') && modifiers == KeyModifiers::CONTROL {
            self.process_pending_jobs();
            return;
        }

        match self.state.active_panel {
            ActivePanel::Sets => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_set(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_set(),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.focus_files_panel(),
                _ => {}
            },
            ActivePanel::Files => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_file_in_set(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_file_in_set(),
                KeyCode::Char('d') => self.set_action_for_selected_file(ActionType::Delete),
                KeyCode::Char('c') => self.initiate_copy_action(),
                KeyCode::Char('s') => self.set_selected_file_as_kept(),
                KeyCode::Char('i') => self.set_action_for_selected_file(ActionType::Ignore),
                KeyCode::Left | KeyCode::Char('h') => self.state.active_panel = ActivePanel::Sets, // Go back to Sets panel
                _ => {}
            },
            ActivePanel::Jobs => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_job(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_job(),
                KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x') => self.remove_selected_job(),
                _ => {}
            },
        }
    }

    fn handle_copy_dest_input_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Enter => {
                let dest_path_str = self.state.current_input.value().to_string();
                self.state.current_input.reset();
                self.state.input_mode = InputMode::Normal;
                if let Some(file_to_copy) = self.state.file_for_copy_move.take() {
                    if !dest_path_str.trim().is_empty() {
                        let dest_path = PathBuf::from(dest_path_str.trim());
                        self.set_action_for_selected_file(ActionType::Copy(dest_path.clone()));
                        self.state.status_message = Some(format!("Marked {} for copy to {}", file_to_copy.path.display(), dest_path.display()));
                    } else {
                        self.state.status_message = Some("Copy cancelled: empty destination path.".to_string());
                    }
                } else {
                     self.state.status_message = Some("Copy cancelled: no file selected.".to_string());
                }
            }
            KeyCode::Esc => {
                self.state.current_input.reset();
                self.state.input_mode = InputMode::Normal;
                self.state.file_for_copy_move = None;
                self.state.status_message = Some("Copy action cancelled.".to_string());
            }
            _ => {
                // Pass the event to tui-input handler
                self.state.current_input.handle_event(&CEvent::Key(key_event));
            }
        }
    }

    fn initiate_copy_action(&mut self) {
        if let Some(selected_file) = self.current_selected_file().cloned() {
            self.state.file_for_copy_move = Some(selected_file);
            self.state.input_mode = InputMode::CopyDestination;
            self.state.current_input.reset(); // Clear previous input
            self.state.status_message = Some("Enter destination path for copy (Enter to confirm, Esc to cancel):".to_string());
        } else {
            self.state.status_message = Some("No file selected to copy.".to_string());
        }
    }

    fn cycle_active_panel(&mut self) {
        self.state.active_panel = match self.state.active_panel {
            ActivePanel::Sets => ActivePanel::Files,
            ActivePanel::Files => ActivePanel::Jobs,
            ActivePanel::Jobs => ActivePanel::Sets,
        };
        log::debug!("Active panel changed to: {:?}", self.state.active_panel);
    }

    fn focus_files_panel(&mut self) {
        if !self.state.duplicate_sets.is_empty() {
            self.state.active_panel = ActivePanel::Files;
        }
    }

    fn select_next_set(&mut self) {
        if !self.state.duplicate_sets.is_empty() {
            self.state.selected_set_index = 
                (self.state.selected_set_index + 1) % self.state.duplicate_sets.len();
            self.state.selected_file_index_in_set = 0;
        }
    }

    fn select_previous_set(&mut self) {
        if !self.state.duplicate_sets.is_empty() {
            if self.state.selected_set_index > 0 {
                self.state.selected_set_index -= 1;
            } else {
                self.state.selected_set_index = self.state.duplicate_sets.len() - 1;
            }
            self.state.selected_file_index_in_set = 0;
        }
    }

    fn select_next_file_in_set(&mut self) {
        if let Some(set) = self.current_selected_set() {
            if !set.files.is_empty() {
                self.state.selected_file_index_in_set = 
                    (self.state.selected_file_index_in_set + 1) % set.files.len();
            }
        }
    }

    fn select_previous_file_in_set(&mut self) {
        if let Some(set) = self.current_selected_set() {
            if !set.files.is_empty() {
                if self.state.selected_file_index_in_set > 0 {
                    self.state.selected_file_index_in_set -= 1;
                } else {
                    self.state.selected_file_index_in_set = set.files.len() - 1;
                }
            }
        }
    }

    fn set_action_for_selected_file(&mut self, action_type: ActionType) {
        if let Some(selected_file_info) = self.current_selected_file().cloned() {
            // Remove any existing job for this file first
            self.state.jobs.retain(|job| job.file_info.path != selected_file_info.path);
            
            // Add the new job
            log::info!("Setting action {:?} for file {:?}", action_type, selected_file_info.path);
            self.state.jobs.push(Job {
                action: action_type.clone(), 
                file_info: selected_file_info.clone(),
            });
            self.state.status_message = Some(format!("Marked {} for {:?}.", selected_file_info.path.file_name().unwrap_or_default().to_string_lossy(), action_type));
        } else {
            self.state.status_message = Some("No file selected to set action.".to_string());
        }
    }

    fn set_selected_file_as_kept(&mut self) {
        if let Some(selected_set) = self.current_selected_set() {
            if let Some(file_to_keep) = selected_set.files.get(self.state.selected_file_index_in_set).cloned() {
                log::info!("User designated {:?} as to be KEPT.", file_to_keep.path);
                self.state.status_message = Some(format!("Marked {} to be KEPT.", file_to_keep.path.file_name().unwrap_or_default().to_string_lossy()));

                // Set selected file to Keep
                self.state.jobs.retain(|job| job.file_info.path != file_to_keep.path); // Remove other jobs for this file
                self.state.jobs.push(Job { action: ActionType::Keep, file_info: file_to_keep.clone() });

                // Set other files in the same set to Delete (unless they are already Ignore)
                for file_in_set in &selected_set.files {
                    if file_in_set.path != file_to_keep.path {
                        // Check if already ignored
                        let is_ignored = self.state.jobs.iter().any(|job| 
                            job.file_info.path == file_in_set.path && job.action == ActionType::Ignore
                        );
                        if !is_ignored {
                            self.state.jobs.retain(|job| job.file_info.path != file_in_set.path); // Remove other jobs
                            self.state.jobs.push(Job { action: ActionType::Delete, file_info: file_in_set.clone() });
                            log::debug!("Auto-marking {:?} for DELETE as another file in set is kept.", file_in_set.path);
                        }
                    }
                }
            } else {
                 self.state.status_message = Some("No file selected to keep.".to_string());
            }
        } else {
            self.state.status_message = Some("No duplicate set selected.".to_string());
        }
    }
    
    fn validate_selection_indices(&mut self) {
        if self.state.duplicate_sets.is_empty() {
            self.state.selected_set_index = 0;
            self.state.selected_file_index_in_set = 0;
            return;
        }
        if self.state.selected_set_index >= self.state.duplicate_sets.len() {
            self.state.selected_set_index = self.state.duplicate_sets.len().saturating_sub(1);
        }

        if let Some(current_set) = self.state.duplicate_sets.get(self.state.selected_set_index) {
            if current_set.files.is_empty() {
                self.state.selected_file_index_in_set = 0;
            } else if self.state.selected_file_index_in_set >= current_set.files.len() {
                self.state.selected_file_index_in_set = current_set.files.len().saturating_sub(1);
            }
        } else {
            self.state.selected_file_index_in_set = 0;
        }

        if self.state.jobs.is_empty() {
            self.state.selected_job_index = 0;
        } else if self.state.selected_job_index >= self.state.jobs.len() {
            self.state.selected_job_index = self.state.jobs.len().saturating_sub(1);
        }
    }

    pub fn current_selected_set(&self) -> Option<&DuplicateSet> {
        self.state.duplicate_sets.get(self.state.selected_set_index)
    }

    pub fn current_selected_file(&self) -> Option<&FileInfo> {
        self.current_selected_set()
            .and_then(|set| set.files.get(self.state.selected_file_index_in_set))
    }

    fn process_pending_jobs(&mut self) -> Result<()> {
        if self.state.jobs.is_empty() {
            self.state.status_message = Some("No jobs to process.".to_string());
            log::info!("No jobs to process.");
            return Ok(());
        }

        log::info!("Processing {} pending jobs...", self.state.jobs.len());
        let mut success_count = 0;
        let mut fail_count = 0;
        let jobs_to_process = self.state.jobs.drain(..).collect::<Vec<_>>(); // Take ownership

        for job in jobs_to_process {
            if job.action == ActionType::Ignore || job.action == ActionType::Keep {
                log::info!("Skipping file {:?} due to {:?} action.", job.file_info.path, job.action);
                continue; 
            }
            log::info!("Executing job: {:?} for file {:?}", job.action, job.file_info.path);
            let result: Result<(), anyhow::Error> = match job.action {
                ActionType::Delete => {
                    match delete_files(&[job.file_info.clone()], false) { 
                        Ok(1) => Ok(()),
                        Ok(count) => Err(anyhow::anyhow!("Delete action affected {} files, expected 1.", count)),
                        Err(e) => Err(e),
                    }
                }
                ActionType::Move(target_dir) => {
                    match move_files(&[job.file_info.clone()], &target_dir, false) {
                        Ok(1) => Ok(()),
                        Ok(count) => Err(anyhow::anyhow!("Move action affected {} files, expected 1.", count)),
                        Err(e) => Err(e),
                    }
                }
                ActionType::Copy(target_dir) => {
                    log::debug!("Attempting to copy {:?} to {:?}", job.file_info.path, target_dir);
                    if !target_dir.exists() {
                        if let Err(e) = std::fs::create_dir_all(&target_dir) {
                            log::error!("Failed to create target directory {:?} for copy: {}", target_dir, e);
                            return Err(e.into());
                        }
                    }
                    let file_name = job.file_info.path.file_name().unwrap_or_default();
                    let mut dest_path = target_dir.join(file_name);
                    let mut counter = 1;
                    while dest_path.exists() {
                        let stem = dest_path.file_stem().unwrap_or_default().to_string_lossy();
                        let ext = dest_path.extension().unwrap_or_default().to_string_lossy();
                        let new_name = format!("{}_copy({}){}{}", 
                                              stem.trim_end_matches(&format!("_copy({})", counter -1 )).trim_end_matches("_copy"),
                                              counter, 
                                              if ext.is_empty() { "" } else { "." }, 
                                              ext);
                        dest_path = target_dir.join(new_name);
                        counter += 1;
                    }
                    std::fs::copy(&job.file_info.path, &dest_path)
                        .map(|_| ()) 
                        .map_err(|e| {
                            log::error!("Failed to copy {:?} to {:?}: {}", job.file_info.path, dest_path, e);
                            anyhow::Error::from(e)
                        })
                }
                ActionType::Keep | ActionType::Ignore => {
                    // This arm should ideally not be reached due to the check above.
                    // If it is, it's an anomaly, but we treat it as a no-op for file system.
                    log::warn!("Reached Keep/Ignore in match arm for file ops: {:?}", job.action);
                    Ok(())
                }
            };

            if result.is_ok() {
                success_count += 1;
                log::info!("Successfully processed job for {:?}", job.file_info.path);
            } else {
                fail_count += 1;
                log::error!("Failed to process job for {:?}: {:?}", job.file_info.path, result.err());
                // Optionally re-add failed jobs, or log them for manual review
            }
        }
        self.state.status_message = Some(format!("Jobs processed. Success: {}, Fail: {}", success_count, fail_count));
        log::info!("Job processing complete. Success: {}, Fail: {}. Jobs list cleared.", success_count, fail_count);
        self.state.selected_job_index = 0; // Reset selection
        Ok(())
    }

    fn select_next_job(&mut self) {
        if !self.state.jobs.is_empty() {
            self.state.selected_job_index = (self.state.selected_job_index + 1) % self.state.jobs.len();
        }
    }

    fn select_previous_job(&mut self) {
        if !self.state.jobs.is_empty() {
            if self.state.selected_job_index > 0 {
                self.state.selected_job_index -= 1;
            } else {
                self.state.selected_job_index = self.state.jobs.len() - 1;
            }
        }
    }

    fn remove_selected_job(&mut self) {
        if !self.state.jobs.is_empty() && self.state.selected_job_index < self.state.jobs.len() {
            let removed_job = self.state.jobs.remove(self.state.selected_job_index);
            log::info!("Removed job: {:?} for file {:?}", removed_job.action, removed_job.file_info.path);
            if self.state.selected_job_index >= self.state.jobs.len() && !self.state.jobs.is_empty() {
                self.state.selected_job_index = self.state.jobs.len() - 1;
            }
             if self.state.jobs.is_empty() {
                self.state.selected_job_index = 0;
            }
            self.state.status_message = Some("Job removed.".to_string());
        } else {
            self.state.status_message = Some("No job selected to remove or jobs list empty.".to_string());
        }
    }
}

type TerminalBackend = CrosstermBackend<Stdout>;

pub fn run_tui_app(cli: &Cli) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cli);
    app.validate_selection_indices(); // Initial validation for sync loaded data if any
    
    let res = run_main_loop(&mut terminal, &mut app, cli.progress); // Pass cli.progress

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Join scan thread if it exists (especially on quit)
    if let Some(handle) = app.scan_thread_join_handle.take() {
        log::debug!("Attempting to join scan thread...");
        if let Err(e) = handle.join() {
            log::error!("Failed to join scan thread: {:?}", e);
        } else {
            log::debug!("Scan thread joined successfully.");
        }
    }

    if let Err(err) = res {
        log::error!("TUI Error: {}", err);
        if log::log_enabled!(log::Level::Debug) {
            if let Some(backtrace) = err.backtrace() {
                 println!("Error in TUI: {}\nStack backtrace:\n{}", err, backtrace);
            } else {
                 println!("Error in TUI: {}", err);
            }
        } else {
            println!("Error in TUI: {}. Run with -vv for more details.", err);
        }
    }

    Ok(())
}

fn run_main_loop(terminal: &mut Terminal<TerminalBackend>, app: &mut App, show_tui_progress: bool) -> Result<()> {
    let tick_rate = Duration::from_millis(100); // Faster tick rate for responsiveness with async msgs
    let mut last_tick = Instant::now();

    loop {
        // Handle messages from scan thread first
        if show_tui_progress { // Only check messages if async scan was started
            app.handle_scan_messages();
        }

        terminal.draw(|f| ui(f, app))?;

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

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn ui(frame: &mut Frame, app: &mut App) {
    if app.state.is_loading && app.scan_rx.is_some() { // Show loading screen only if async scan is active
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Length(3),
                Constraint::Percentage(40),
            ])
            .split(frame.size());
        
        let loading_block = Block::default().title("Loading...").borders(Borders::ALL);
        let text = Paragraph::new(app.state.loading_message.as_str())
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        
        let area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(20), Constraint::Percentage(60), Constraint::Percentage(20)]).split(chunks[1])[1]; // Middle 60% of the middle chunk

        frame.render_widget(Clear, area); // Clear the area for the centered text
        frame.render_widget(text.block(loading_block), area);
    } else {
        // Main UI (3 panels + status bar)
        let (content_chunk, status_chunk) = {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0), 
                    Constraint::Length(if app.state.input_mode == InputMode::Normal { 1 } else { 3 }), 
                ])
                .split(frame.size());
            (chunks[0], chunks[1])
        };

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33), 
                Constraint::Percentage(34), 
                Constraint::Percentage(33), 
            ].as_ref())
            .split(content_chunk); 

        // Helper to create a block with a title and border, highlighting if active
        let create_block = |title: &str, is_active: bool| {
            let base_style = if is_active { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::White) };
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, base_style))
                .border_style(base_style)
        };
        let sets_block_title = format!("Duplicate Sets ({}/{}) (Tab)", app.state.selected_set_index.saturating_add(1).min(app.state.duplicate_sets.len()), app.state.duplicate_sets.len());
        let sets_block = create_block(&sets_block_title, app.state.active_panel == ActivePanel::Sets && app.state.input_mode == InputMode::Normal);
        let set_items: Vec<ListItem> = app.state.duplicate_sets.iter().map(|set| {
            let content = Line::from(Span::styled(
                format!("Hash: {}... ({} files, {})",
                    set.hash.chars().take(8).collect::<String>(),
                    set.files.len(),
                    format_size(set.size, DECIMAL)),
                Style::default(),
            ));
            ListItem::new(content)
        }).collect();
        let sets_list = List::new(set_items)
            .block(sets_block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::Blue))
            .highlight_symbol(">> ");
        let mut sets_list_state = ListState::default();
        if !app.state.duplicate_sets.is_empty() {
            sets_list_state.select(Some(app.state.selected_set_index));
        }
        frame.render_stateful_widget(sets_list, main_chunks[0], &mut sets_list_state);
        let (files_block_title, file_items) = 
            if let Some(selected_set) = app.current_selected_set() {
                let title = format!("Files ({}/{}) (s:keep d:del c:copy i:ign h:back)", 
                                    app.state.selected_file_index_in_set.saturating_add(1).min(selected_set.files.len()), 
                                    selected_set.files.len());
                let items: Vec<ListItem> = selected_set.files.iter().map(|file_info| {
                    let mut style = Style::default();
                    let mut prefix = "   ";
                    if let Some(job) = app.state.jobs.iter().find(|j| j.file_info.path == file_info.path) {
                        match job.action {
                            ActionType::Keep => { style = style.fg(Color::Green).add_modifier(Modifier::BOLD); prefix = "[K]"; }
                            ActionType::Delete => { style = style.fg(Color::Red).add_modifier(Modifier::STRIKETHROUGH); prefix = "[D]"; }
                            ActionType::Copy(_) => { style = style.fg(Color::Cyan); prefix = "[C]"; }
                            ActionType::Move(_) => { style = style.fg(Color::Magenta); prefix = "[M]"; }
                            ActionType::Ignore => { style = style.fg(Color::DarkGray); prefix = "[I]"; }
                        }
                    } else {
                        if let Ok((default_kept, _)) = file_utils::determine_action_targets(selected_set, app.state.default_selection_strategy) {
                            if default_kept.path == file_info.path {
                                style = style.fg(Color::Green);
                                prefix = "[k]";
                            }
                        }
                    }
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{} ", prefix), style),
                        Span::styled(file_info.path.display().to_string(), style)
                    ]))
                }).collect();
                (title, items)
            } else {
                ("Files (0/0)".to_string(), vec![ListItem::new("No set selected or set is empty")])
            };
        let files_block = create_block(&files_block_title, app.state.active_panel == ActivePanel::Files && app.state.input_mode == InputMode::Normal);
        let files_list = List::new(file_items)
            .block(files_block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::DarkGray))
            .highlight_symbol("> ");

        let mut files_list_state = ListState::default();
        if app.current_selected_set().map_or(false, |s| !s.files.is_empty()) {
            files_list_state.select(Some(app.state.selected_file_index_in_set));
        }
        frame.render_stateful_widget(files_list, main_chunks[1], &mut files_list_state);
        let jobs_block_title = format!("Jobs ({}) (Ctrl+E: Exec, x:del)", app.state.jobs.len());
        let jobs_block = create_block(&jobs_block_title, app.state.active_panel == ActivePanel::Jobs && app.state.input_mode == InputMode::Normal);
        let job_items: Vec<ListItem> = app.state.jobs.iter().map(|job| {
            let action_str = match &job.action {
                ActionType::Keep => "KEEP".to_string(),
                ActionType::Delete => "DELETE".to_string(),
                ActionType::Move(dest) => format!("MOVE to {}", dest.display()),
                ActionType::Copy(dest) => format!("COPY to {}", dest.display()),
                ActionType::Ignore => "IGNORE".to_string(),
            };
            let content = Line::from(Span::raw(format!("{} - {:?}", action_str, job.file_info.path.file_name().unwrap_or_default())));
            ListItem::new(content)
        }).collect();
        let jobs_list_widget = List::new(job_items)
            .block(jobs_block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::Purple))
            .highlight_symbol(">> ");
        let mut jobs_list_state = ListState::default();
        if !app.state.jobs.is_empty() {
            jobs_list_state.select(Some(app.state.selected_job_index));
        }
        frame.render_stateful_widget(jobs_list_widget, main_chunks[2], &mut jobs_list_state);

        // Status Bar / Input Area
        match app.state.input_mode {
            InputMode::Normal => {
                let status_text = app.state.status_message.as_deref().unwrap_or("q:quit | Tab:cycle | Arrows/jk:nav | s:keep d:del c:copy i:ign | Ctrl+E:exec | x:del job");
                let status_bar = Paragraph::new(status_text)
                    .style(Style::default().fg(Color::LightCyan))
                    .alignment(Alignment::Left);
                frame.render_widget(status_bar, status_chunk);
            }
            InputMode::CopyDestination => { 
                let input_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Length(1)]).split(status_chunk);
                let prompt_text = app.state.status_message.as_deref().unwrap_or("Enter destination path for copy (Enter:confirm, Esc:cancel):");
                let prompt_p = Paragraph::new(prompt_text).fg(Color::Yellow);
                frame.render_widget(prompt_p, input_chunks[0]);
                let input_field = Paragraph::new(app.state.current_input.value())
                    .block(Block::default().borders(Borders::TOP).title("Path").border_style(Style::default().fg(Color::Yellow)))
                    .fg(Color::White);
                frame.render_widget(input_field, input_chunks[1]);
                frame.set_cursor(
                    input_chunks[1].x + app.state.current_input.visual_cursor() as u16 + 1, 
                    input_chunks[1].y + 1 
                );
             }
        }
    }
}

// TODO: Define Job struct and ActionType enum for the right panel
// enum ActionType { Delete, Copy, Move }
// struct Job { action: ActionType, file_info: FileInfo, destination: Option<PathBuf> } 