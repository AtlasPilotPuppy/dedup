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
    Settings, // New mode for settings
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

    // Modifiable scan settings
    pub current_algorithm: String,
    pub current_parallel: Option<usize>,
    pub rescan_needed: bool, // Flag to indicate if settings changed and rescan is advised

    // Settings Menu State
    pub selected_setting_category_index: usize, // 0: Strategy, 1: Algorithm, 2: Parallelism
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
    scan_tx: Option<std_mpsc::Sender<ScanMessage>>, // Added sender to be stored for rescans
    cli_config: Cli, // Store the initial CLI config
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
            current_algorithm: cli_args.algorithm.clone(), // Initialize from Cli
            current_parallel: cli_args.parallel,        // Initialize from Cli
            rescan_needed: false,
            selected_setting_category_index: 0, // Default to strategy
        };

        let (tx, rx) = std_mpsc::channel::<ScanMessage>();
        let scan_join_handle: Option<std_thread::JoinHandle<()>> = if cli_args.progress {
            let mut current_cli_for_scan = cli_args.clone();
            current_cli_for_scan.algorithm = app_state.current_algorithm.clone();
            current_cli_for_scan.parallel = app_state.current_parallel;

            let thread_tx = tx.clone();
            let handle = std_thread::spawn(move || {
                log::info!("[ScanThread] Starting initial duplicate scan...");
                match file_utils::find_duplicate_files_with_progress(&current_cli_for_scan, thread_tx.clone()) {
                    Ok(result) => {
                        if thread_tx.send(ScanMessage::Completed(Ok(result))).is_err() {
                            log::error!("[ScanThread] Failed to send completion message to TUI.");
                        }
                    }
                    Err(e) => {
                        if thread_tx.send(ScanMessage::Error(e.to_string())).is_err() {
                            log::error!("[ScanThread] Failed to send error message to TUI.");
                        }
                    }
                }
                log::info!("[ScanThread] Initial scan finished.");
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
            scan_tx: if cli_args.progress { Some(tx) } else { None }, // Store sender only if async
            cli_config: cli_args.clone(), // Store a clone of the initial Cli
        }
    }

    // Method to trigger a rescan
    fn trigger_rescan(&mut self) {
        if self.state.is_loading && self.scan_thread_join_handle.is_some() {
            self.state.status_message = Some("Scan already in progress.".to_string());
            return;
        }

        // Attempt to join the previous scan thread if it exists
        if let Some(handle) = self.scan_thread_join_handle.take() {
            log::debug!("Attempting to join previous scan thread before rescan...");
            if let Err(e) = handle.join() {
                log::error!("Failed to join previous scan thread: {:?}", e);
                // Decide if we should proceed or not, for now, we proceed cautiously
            }
        }

        self.state.duplicate_sets.clear();
        self.state.jobs.clear();
        self.state.selected_set_index = 0;
        self.state.selected_file_index_in_set = 0;
        self.state.selected_job_index = 0;
        self.state.is_loading = true;
        self.state.loading_message = "Rescanning with current settings...".to_string();
        self.state.status_message = Some("Starting rescan...".to_string());
        self.state.rescan_needed = false; // Reset flag as we are acting on it

        let mut current_cli_for_scan = self.cli_config.clone(); // Use stored cli_config
        current_cli_for_scan.algorithm = self.state.current_algorithm.clone();
        current_cli_for_scan.parallel = self.state.current_parallel;
        // Note: We always use progress for TUI internal scans regardless of initial cli.progress
        // find_duplicate_files_with_progress requires a tx channel.
        // Ensure scan_tx is Some.
        if self.scan_tx.is_none() {
            let (tx, rx) = std_mpsc::channel::<ScanMessage>();
            self.scan_tx = Some(tx);
            self.scan_rx = Some(rx); // Also need to re-assign rx if tx was None
        }

        if let Some(tx_cloned) = self.scan_tx.clone() { // Ensure tx exists
            let handle = std_thread::spawn(move || {
                log::info!("[ScanThread] Starting rescan for duplicates...");
                match file_utils::find_duplicate_files_with_progress(&current_cli_for_scan, tx_cloned.clone()) {
                    Ok(result) => {
                        if tx_cloned.send(ScanMessage::Completed(Ok(result))).is_err() {
                            log::error!("[ScanThread] Failed to send rescan completion message to TUI.");
                        }
                    }
                    Err(e) => {
                        if tx_cloned.send(ScanMessage::Error(e.to_string())).is_err() {
                            log::error!("[ScanThread] Failed to send rescan error message to TUI.");
                        }
                    }
                }
                log::info!("[ScanThread] Rescan finished.");
            });
            self.scan_thread_join_handle = Some(handle);
        } else {
            // This case should ideally not be reached if TUI always uses progress/async scan
            log::error!("Failed to start rescan: No sender channel available.");
            self.state.is_loading = false;
            self.state.loading_message = "Error: Could not start rescan.".to_string();
            self.state.status_message = Some("Rescan failed to start. Check logs.".to_string());
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

        match self.state.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key_event),
            InputMode::CopyDestination => self.handle_copy_dest_input_key(key_event),
            InputMode::Settings => self.handle_settings_mode_key(key_event),
        }
        self.validate_selection_indices(); // Ensure selections are valid after any action
    }

    fn handle_normal_mode_key(&mut self, key_event: KeyEvent) {
        let key_code = key_event.code;
        let modifiers = key_event.modifiers;

        // General shortcuts first
        if key_code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }
        if key_code == KeyCode::Tab {
            self.cycle_active_panel();
            return;
        }
        if key_code == KeyCode::Char('e') && modifiers == KeyModifiers::CONTROL {
            if let Err(e) = self.process_pending_jobs() {
                self.state.status_message = Some(format!("Error processing jobs: {}", e));
            }
            return;
        }
        if key_code == KeyCode::Char('r') && modifiers == KeyModifiers::CONTROL {
            self.trigger_rescan();
            return;
        }
        if key_code == KeyCode::Char('s') && modifiers == KeyModifiers::CONTROL { // Ctrl+S for Settings
            self.state.input_mode = InputMode::Settings;
            self.state.status_message = Some("Entered settings mode. Esc to exit.".to_string());
            // TODO: Initialize settings focus, e.g., self.state.selected_setting_index = 0;
            return;
        }

        // Then panel-specific shortcuts
        match self.state.active_panel {
            ActivePanel::Sets => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_set(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_set(),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.focus_files_panel(),
                KeyCode::Char('d') => self.mark_set_for_deletion(),
                _ => {}
            },
            ActivePanel::Files => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_file_in_set(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_file_in_set(),
                KeyCode::Char('d') => self.set_action_for_selected_file(ActionType::Delete),
                KeyCode::Char('c') => self.initiate_copy_action(),
                KeyCode::Char('s') => self.set_selected_file_as_kept(),
                KeyCode::Char('i') => self.set_action_for_selected_file(ActionType::Ignore),
                KeyCode::Left | KeyCode::Char('h') => self.state.active_panel = ActivePanel::Sets,
                _ => {}
            },
            ActivePanel::Jobs => match key_code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_job(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_job(),
                KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x') => self.remove_selected_job(),
                _ => {}
            },
        }
        // self.validate_selection_indices(); // Already called in on_key
    }

    fn handle_settings_mode_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Normal;
                if self.state.rescan_needed {
                    self.state.status_message = Some("Exited settings. Ctrl+R to apply changes.".to_string());
                } else {
                    self.state.status_message = Some("Exited settings mode.".to_string());
                }
            }
            KeyCode::Up => {
                self.state.selected_setting_category_index = self.state.selected_setting_category_index.saturating_sub(1);
            }
            KeyCode::Down => {
                self.state.selected_setting_category_index = (self.state.selected_setting_category_index + 1).min(2); // 3 categories (0,1,2)
            }
            // Strategy selection keys (n, o, s, l)
            KeyCode::Char('n') if self.state.selected_setting_category_index == 0 => { 
                self.state.default_selection_strategy = SelectionStrategy::NewestModified;
                self.state.status_message = Some("Strategy: Newest Modified".to_string());
            }
            KeyCode::Char('o') if self.state.selected_setting_category_index == 0 => { 
                self.state.default_selection_strategy = SelectionStrategy::OldestModified;
                self.state.status_message = Some("Strategy: Oldest Modified".to_string());
            }
            KeyCode::Char('s') if self.state.selected_setting_category_index == 0 => { 
                self.state.default_selection_strategy = SelectionStrategy::ShortestPath;
                self.state.status_message = Some("Strategy: Shortest Path".to_string());
            }
            KeyCode::Char('l') if self.state.selected_setting_category_index == 0 => { 
                self.state.default_selection_strategy = SelectionStrategy::LongestPath;
                self.state.status_message = Some("Strategy: Longest Path".to_string());
            }
            // Algorithm selection keys (m, a, b)
            KeyCode::Char('m') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "md5".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: md5 (Rescan needed)".to_string());
            }
            KeyCode::Char('a') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "sha256".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: sha256 (Rescan needed)".to_string());
            }
            KeyCode::Char('b') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "blake3".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: blake3 (Rescan needed)".to_string());
            }
            // Parallelism adjustment keys (+, -, 0-9)
            KeyCode::Char('0') if self.state.selected_setting_category_index == 2 => {
                self.state.current_parallel = None; // None signifies auto
                self.state.rescan_needed = true;
                self.state.status_message = Some("Parallel Cores: Auto (Rescan needed)".to_string());
            }
            KeyCode::Char(c @ '1'..='9') if self.state.selected_setting_category_index == 2 => {
                // Simple single digit for now. Could extend to multi-digit input.
                let cores = c.to_digit(10).map(|d| d as usize);
                if self.state.current_parallel != cores {
                    self.state.current_parallel = cores;
                    self.state.rescan_needed = true;
                    self.state.status_message = Some(format!("Parallel Cores: {} (Rescan needed)", c));
                }
            }
            KeyCode::Char('+') if self.state.selected_setting_category_index == 2 => {
                let current_val = self.state.current_parallel.unwrap_or(0);
                // Cap at num_cpus or a reasonable max like 16 if num_cpus is too high/unavailable?
                // For simplicity, let's just increment, max 16 for now.
                let new_val = (current_val + 1).min(16);
                if self.state.current_parallel != Some(new_val) {
                    self.state.current_parallel = Some(new_val);
                    self.state.rescan_needed = true;
                    self.state.status_message = Some(format!("Parallel Cores: {} (Rescan needed)", new_val));
                }
            }
            KeyCode::Char('-') if self.state.selected_setting_category_index == 2 => {
                let current_val = self.state.current_parallel.unwrap_or(1); // If auto (None), treat as 1 for decrement start
                if current_val > 1 { // Minimum 1 core
                    let new_val = current_val - 1;
                     if self.state.current_parallel != Some(new_val) {
                        self.state.current_parallel = Some(new_val);
                        self.state.rescan_needed = true;
                        self.state.status_message = Some(format!("Parallel Cores: {} (Rescan needed)", new_val));
                    }
                } else if current_val == 1 && self.state.current_parallel.is_some() { // Allow going from 1 to Auto (None)
                    self.state.current_parallel = None;
                    self.state.rescan_needed = true;
                    self.state.status_message = Some("Parallel Cores: Auto (Rescan needed)".to_string());
                }
            }
            _ => {}
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
        let set_index = self.state.selected_set_index;
        let file_index_in_set = self.state.selected_file_index_in_set;

        if let Some(file_to_keep) = self.state.duplicate_sets.get(set_index)
                                    .and_then(|s| s.files.get(file_index_in_set).cloned()) {
            
            log::info!("User designated {:?} as to be KEPT.", file_to_keep.path);
            self.state.status_message = Some(format!("Marked {} to be KEPT.", file_to_keep.path.file_name().unwrap_or_default().to_string_lossy()));

            // Set selected file to Keep
            self.state.jobs.retain(|job| job.file_info.path != file_to_keep.path); 
            self.state.jobs.push(Job { action: ActionType::Keep, file_info: file_to_keep.clone() });

            let current_set_files_clone = self.state.duplicate_sets.get(set_index).map_or(Vec::new(), |s| s.files.clone());

            for file_in_set in current_set_files_clone {
                if file_in_set.path != file_to_keep.path {
                    let is_ignored = self.state.jobs.iter().any(|job| 
                        job.file_info.path == file_in_set.path && job.action == ActionType::Ignore
                    );
                    if !is_ignored {
                        self.state.jobs.retain(|job| job.file_info.path != file_in_set.path); 
                        self.state.jobs.push(Job { action: ActionType::Delete, file_info: file_in_set.clone() });
                        log::debug!("Auto-marking {:?} for DELETE as another file in set is kept.", file_in_set.path);
                    }
                }
            }
        } else {
            self.state.status_message = Some("No file/set selected or available to keep.".to_string());
        }
    }
    
    fn mark_set_for_deletion(&mut self) {
        if let Some(selected_set) = self.current_selected_set().cloned() { // Clone to avoid borrow issues
            if selected_set.files.len() < 2 {
                self.state.status_message = Some("Set has less than 2 files, no action taken.".to_string());
                return;
            }

            match file_utils::determine_action_targets(&selected_set, self.state.default_selection_strategy) {
                Ok((kept_file, files_to_delete)) => {
                    let kept_file_path = kept_file.path.clone();
                    let mut files_marked_for_delete = 0;

                    // First, remove any existing jobs for files in this set
                    self.state.jobs.retain(|job| {
                        !selected_set.files.iter().any(|f_in_set| f_in_set.path == job.file_info.path)
                    });

                    // Add Keep job for the determined file
                    self.state.jobs.push(Job {
                        action: ActionType::Keep,
                        file_info: kept_file.clone(),
                    });
                    log::info!("Auto-marking {:?} to KEEP based on strategy {:?}.", kept_file.path, self.state.default_selection_strategy);

                    // Add Delete jobs for all other files
                    for file_to_delete in files_to_delete {
                        // Double check it's not the one we decided to keep (should be handled by determine_action_targets)
                        if file_to_delete.path != kept_file_path {
                            self.state.jobs.push(Job {
                                action: ActionType::Delete,
                                file_info: file_to_delete.clone(),
                            });
                            files_marked_for_delete += 1;
                            log::info!("Auto-marking {:?} for DELETE in set.", file_to_delete.path);
                        }
                    }
                    self.state.status_message = Some(format!("Marked {} files for DELETE, 1 to KEEP in current set.", files_marked_for_delete));
                }
                Err(e) => {
                    self.state.status_message = Some(format!("Error determining actions for set: {}", e));
                    log::error!("Could not determine action targets for set deletion: {}", e);
                }
            }
        } else {
            self.state.status_message = Some("No set selected.".to_string());
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
            let mut backtrace_output = "(backtrace not available or disabled)".to_string();
            
            // Explicitly wrap the backtrace reference in Some() because err.backtrace() returns &Backtrace
            let an_option_of_backtrace: Option<&std::backtrace::Backtrace> = Some(err.backtrace());

            // Now match on this explicitly typed Option
            if let Some(bt_ref) = an_option_of_backtrace { // bt_ref should be &std::backtrace::Backtrace
                backtrace_output = format!("Stack backtrace:\n{}", bt_ref); // &std::backtrace::Backtrace implements Display
            }
            println!("Error in TUI: {}\n{}", err, backtrace_output);
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
    } else if app.state.input_mode == InputMode::Settings {
        // Basic placeholder for settings UI
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(10),   // Settings options
                Constraint::Length(1), // Hint
            ])
            .split(frame.size());

        let title = Paragraph::new("--- Settings Menu ---")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Settings (Ctrl+S to enter/Esc to exit)"));
        frame.render_widget(title, chunks[0]);

        let mut strategy_style = Style::default();
        let mut algo_style = Style::default();
        let mut parallel_style = Style::default();

        match app.state.selected_setting_category_index {
            0 => strategy_style = strategy_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            1 => algo_style = algo_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            2 => parallel_style = parallel_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            _ => {}
        }

        let settings_text = vec![
            Line::from(Span::styled(format!("1. File Selection Strategy: {:?}", app.state.default_selection_strategy), strategy_style)),
            Line::from(Span::styled(format!("   (n:newest, o:oldest, s:shortest, l:longest)"), strategy_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("2. Hashing Algorithm: {}", app.state.current_algorithm), algo_style)),
            Line::from(Span::styled(format!("   (m:md5, a:sha256, b:blake3)"), algo_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("3. Parallel Cores: {}", app.state.current_parallel.map_or("Auto".to_string(), |c| c.to_string())), parallel_style)),
            Line::from(Span::styled(format!("   (0 for auto, 1-N for specific count. Use +/- or numbers)"), parallel_style)),
            Line::from(Span::raw("")),
            Line::from(Span::raw(if app.state.rescan_needed {
                "[!] Settings changed. Press Ctrl+R to rescan for changes to take effect."
            } else {
                "No pending setting changes."
            })),
        ];
        let settings_paragraph = Paragraph::new(settings_text)
            .block(Block::default().borders(Borders::ALL).title("Options"))
            .wrap({ Wrap { trim: true } });
        frame.render_widget(settings_paragraph, chunks[1]);

        let hint = Paragraph::new("Esc: Exit Settings | Use indicated keys to change values.")
            .alignment(Alignment::Center);
        frame.render_widget(hint, chunks[2]);

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
        let create_block = |title_string: String, is_active: bool| {
            let base_style = if is_active { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::White) };
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title_string, base_style))
                .border_style(base_style)
        };

        // Left Panel: Duplicate Sets
        let sets_panel_title_string = format!("Duplicate Sets ({}/{}) (Tab)", app.state.selected_set_index.saturating_add(1).min(app.state.duplicate_sets.len()), app.state.duplicate_sets.len());
        let sets_block = create_block(sets_panel_title_string, app.state.active_panel == ActivePanel::Sets && app.state.input_mode == InputMode::Normal);
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

        // Middle Panel: Files in Selected Set
        let (files_panel_title_string, file_items) = 
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
                            ActionType::Delete => { style = style.fg(Color::Red).add_modifier(Modifier::CROSSED_OUT); prefix = "[D]"; }
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
        let files_block = create_block(files_panel_title_string, app.state.active_panel == ActivePanel::Files && app.state.input_mode == InputMode::Normal);
        let files_list = List::new(file_items)
            .block(files_block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::DarkGray))
            .highlight_symbol("> ");

        let mut files_list_state = ListState::default();
        if app.current_selected_set().map_or(false, |s| !s.files.is_empty()) {
            files_list_state.select(Some(app.state.selected_file_index_in_set));
        }
        frame.render_stateful_widget(files_list, main_chunks[1], &mut files_list_state);

        // Right Panel: Jobs
        let jobs_panel_title_string = format!("Jobs ({}) (Ctrl+E: Exec, x:del)", app.state.jobs.len());
        let jobs_block = create_block(jobs_panel_title_string, app.state.active_panel == ActivePanel::Jobs && app.state.input_mode == InputMode::Normal);
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
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::Magenta))
            .highlight_symbol(">> ");
        let mut jobs_list_state = ListState::default();
        if !app.state.jobs.is_empty() {
            jobs_list_state.select(Some(app.state.selected_job_index));
        }
        frame.render_stateful_widget(jobs_list_widget, main_chunks[2], &mut jobs_list_state);

        // Status Bar / Input Area
        match app.state.input_mode {
            InputMode::Normal => {
                let status_text = app.state.status_message.as_deref().unwrap_or("q:quit | Tab:cycle | Arrows/jk:nav | s:keep d:del c:copy i:ign | Ctrl+E:exec | Ctrl+R:rescan | Ctrl+S:settings | x:del job");
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
            InputMode::Settings => {
                // The Settings mode has its own full-screen UI, so no specific status bar here.
                // The hints are part of the settings_paragraph and hint Paragraph already rendered.
            }
        }
    }
}

// TODO: Define Job struct and ActionType enum for the right panel
// enum ActionType { Delete, Copy, Move }
// struct Job { action: ActionType, file_info: FileInfo, destination: Option<PathBuf> } 