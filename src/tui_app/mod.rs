use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::io::{stdout, Stdout};
use std::time::{Duration, Instant};
use std::sync::mpsc as std_mpsc; // Alias to avoid conflict if crate::mpsc is used elsewhere
use std::thread as std_thread; // Alias for clarity
use num_cpus; // For displaying actual core count in auto mode
use std::collections::HashMap; // For grouping
use std::path::{Path, PathBuf}; // Ensure Path is imported here
use tui_input::backend::crossterm::EventHandler; // For tui-input
use tui_input::Input;
use humansize::{format_size, DECIMAL};

use crate::Cli;
use crate::file_utils::{self, DuplicateSet, FileInfo, SelectionStrategy, delete_files, move_files, SortCriterion, SortOrder}; // Added SortCriterion, SortOrder

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
    Help, // New mode for help screen
}

// ---- New structs for parent folder grouping ----
#[derive(Debug, Clone)]
pub struct ParentFolderGroup {
    pub path: PathBuf,
    pub sets: Vec<DuplicateSet>,
    pub is_expanded: bool,
}

#[derive(Debug, Clone)]
pub enum DisplayListItem {
    Folder {
        path: PathBuf,
        is_expanded: bool,
        set_count: usize,
    },
    SetEntry {
        set_hash_preview: String,
        set_total_size: u64,
        file_count_in_set: usize,
        original_group_index: usize,
        original_set_index_in_group: usize,
        indent: bool,
    },
}
// ---- End new structs ----

#[derive(Debug)]
pub struct AppState {
    pub grouped_data: Vec<ParentFolderGroup>,
    pub display_list: Vec<DisplayListItem>,
    pub selected_display_list_index: usize,
    pub selected_file_index_in_set: usize,
    pub selected_job_index: usize,
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
    pub selected_setting_category_index: usize, // 0: Strategy, 1: Algorithm, 2: Parallelism, 3: Sort Criterion, 4: Sort Order
    pub current_sort_criterion: SortCriterion, // New for sorting
    pub current_sort_order: SortOrder,         // New for sorting
    pub sort_settings_changed: bool,           // Flag if sorting needs re-application
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
        let initial_status = if cli_args.progress { // Check original progress flag for initial scan message
            "Initializing scan...".to_string()
        } else {
            "Loading... (performing initial scan)".to_string()
        };

        let mut app_state = AppState {
            grouped_data: Vec::new(),
            display_list: Vec::new(),
            selected_display_list_index: 0,
            selected_file_index_in_set: 0,
            selected_job_index: 0,
            jobs: Vec::new(),
            active_panel: ActivePanel::Sets,
            default_selection_strategy: strategy,
            status_message: None,
            input_mode: InputMode::Normal,
            current_input: Input::default(),
            file_for_copy_move: None,
            is_loading: true, // Always start in loading state, scan will update
            loading_message: initial_status,
            current_algorithm: cli_args.algorithm.clone(),
            current_parallel: cli_args.parallel,
            rescan_needed: false,
            selected_setting_category_index: 0, 
            current_sort_criterion: cli_args.sort_by,   // Initialize from Cli
            current_sort_order: cli_args.sort_order,    // Initialize from Cli
            sort_settings_changed: false, 
        };

        // Determine if initial scan is async or sync based on original cli.progress
        let perform_async_scan = cli_args.progress; 

        let (tx, rx) = std_mpsc::channel::<ScanMessage>();
        let scan_join_handle: Option<std_thread::JoinHandle<()>> = if perform_async_scan {
            let mut current_cli_for_scan = cli_args.clone(); // Clone entire cli_args
            // These are already set from cli_args, but ensure they are what scan thread uses
            current_cli_for_scan.algorithm = app_state.current_algorithm.clone();
            current_cli_for_scan.parallel = app_state.current_parallel;
            current_cli_for_scan.sort_by = app_state.current_sort_criterion;
            current_cli_for_scan.sort_order = app_state.current_sort_order;

            let thread_tx = tx.clone();
            let handle = std_thread::spawn(move || {
                log::info!("[ScanThread] Starting initial duplicate scan...");
                match file_utils::find_duplicate_files_with_progress(&current_cli_for_scan, thread_tx.clone()) {
                    Ok(raw_sets) => {
                        if thread_tx.send(ScanMessage::Completed(Ok(raw_sets))).is_err() {
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
                Ok(raw_sets) => {
                    let (grouped_data, display_list) = App::process_raw_sets_into_grouped_view(raw_sets, true); // Default expanded
                    app_state.grouped_data = grouped_data;
                    app_state.display_list = display_list;
                    if app_state.display_list.is_empty() {
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
        
        if app_state.selected_display_list_index >= app_state.display_list.len() && !app_state.display_list.is_empty() {
            app_state.selected_display_list_index = app_state.display_list.len().saturating_sub(1);
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

    fn process_raw_sets_into_grouped_view(sets: Vec<DuplicateSet>, default_expanded: bool) -> (Vec<ParentFolderGroup>, Vec<DisplayListItem>) {
        let mut parent_map: HashMap<PathBuf, Vec<DuplicateSet>> = HashMap::new();
        for set in sets {
            if let Some(first_file) = set.files.first() {
                let parent = first_file.path.parent().unwrap_or_else(|| Path::new("/")).to_path_buf();
                parent_map.entry(parent).or_default().push(set);
            }
        }

        let mut grouped_data: Vec<ParentFolderGroup> = parent_map.into_iter()
            .map(|(path, sets_in_group)| ParentFolderGroup {
                path,
                sets: sets_in_group,
                is_expanded: default_expanded,
            })
            .collect();

        grouped_data.sort_by(|a, b| a.path.cmp(&b.path));
        for group in &mut grouped_data {
            group.sets.sort_by(|a,b| a.hash.cmp(&b.hash)); // Ensure consistent order of sets within a folder
        }

        let display_list = App::build_display_list_from_grouped_data(&grouped_data);
        (grouped_data, display_list)
    }

    fn build_display_list_from_grouped_data(grouped_data: &[ParentFolderGroup]) -> Vec<DisplayListItem> {
        let mut display_list = Vec::new();
        for (group_idx, group) in grouped_data.iter().enumerate() {
            display_list.push(DisplayListItem::Folder {
                path: group.path.clone(),
                is_expanded: group.is_expanded,
                set_count: group.sets.len(),
            });
            if group.is_expanded {
                for (set_idx, set_item) in group.sets.iter().enumerate() {
                    display_list.push(DisplayListItem::SetEntry {
                        set_hash_preview: set_item.hash.chars().take(8).collect(),
                        set_total_size: set_item.size,
                        file_count_in_set: set_item.files.len(),
                        original_group_index: group_idx,
                        original_set_index_in_group: set_idx,
                        indent: true,
                    });
                }
            }
        }
        display_list
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

        self.state.grouped_data.clear();
        self.state.display_list.clear();
        self.state.jobs.clear();
        self.state.selected_display_list_index = 0;
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
                    Ok(raw_sets) => {
                        if tx_cloned.send(ScanMessage::Completed(Ok(raw_sets))).is_err() {
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
                            Ok(raw_sets) => {
                                let (grouped_data, display_list) = App::process_raw_sets_into_grouped_view(raw_sets, true); // Default expanded
                                self.state.grouped_data = grouped_data;
                                self.state.display_list = display_list;
                                if self.state.display_list.is_empty() {
                                    self.state.status_message = Some("Scan complete. No duplicates found.".to_string());
                                } else {
                                    self.state.status_message = Some(format!("Scan complete. Found {} display items.", self.state.display_list.len()));
                                }
                            }
                            Err(e) => {
                                self.state.loading_message = format!("Error during scan: {}", e);
                                self.state.status_message = Some("Scan failed: Check logs.".to_string());
                                log::error!("Scan thread reported error: {}", e);
                            }
                        }
                        self.rebuild_display_list(); // This ensures validate_selection_indices is called
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
            InputMode::Help => self.handle_help_mode_key(key_event),
        }
        self.validate_selection_indices(); // Ensure selections are valid after any action
    }

    fn handle_normal_mode_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('h') => {
                self.state.input_mode = InputMode::Help;
                self.state.status_message = Some("Displaying Help. Esc to exit.".to_string());
            }
            KeyCode::Char('a') => {
                // Select all files in the current set
                if let Some(selected) = self.state.display_list.get(self.state.selected_display_list_index) {
                    match selected {
                        DisplayListItem::SetEntry { original_group_index, original_set_index_in_group, .. } => {
                            // Get a copy of the files we need to process
                            let files_to_process = if let Some(group) = self.state.grouped_data.get(*original_group_index) {
                                group.sets[*original_set_index_in_group].files.clone()
                            } else {
                                Vec::new()
                            };
                            
                            // Now process the files
                            for _file in files_to_process {
                                self.set_action_for_selected_file(ActionType::Keep);
                            }
                            self.state.status_message = Some("All files in set marked to keep".to_string());
                        }
                        DisplayListItem::Folder { .. } => {
                            // Get a copy of all files in the folder
                            let files_to_process = if let Some(group) = self.state.grouped_data.get(self.state.selected_display_list_index) {
                                group.sets.iter()
                                    .flat_map(|set| set.files.clone())
                                    .collect::<Vec<_>>()
                            } else {
                                Vec::new()
                            };
                            
                            // Now process the files
                            for _file in files_to_process {
                                self.set_action_for_selected_file(ActionType::Keep);
                            }
                            self.state.status_message = Some("All files in folder marked to keep".to_string());
                        }
                    }
                }
            }
            KeyCode::Tab => {
                self.cycle_active_panel();
            }
            KeyCode::Char('e') => {
                if let Err(e) = self.process_pending_jobs() {
                    self.state.status_message = Some(format!("Error processing jobs: {}", e));
                }
            }
            KeyCode::Char('r') => {
                self.trigger_rescan();
            }
            KeyCode::Char('s') => {
                self.state.input_mode = InputMode::Settings;
                self.state.status_message = Some("Entered settings mode. Esc to exit.".to_string());
            }
            KeyCode::Char('d') => {
                self.mark_set_for_deletion();
            }
            KeyCode::Char('i') => {
                self.set_action_for_selected_file(ActionType::Ignore);
            }
            KeyCode::Char('c') => {
                self.initiate_copy_action();
            }
            KeyCode::Char('k') => {
                self.set_selected_file_as_kept();
            }
            KeyCode::Up => {
                match self.state.active_panel {
                    ActivePanel::Sets => self.select_previous_set(),
                    ActivePanel::Files => self.select_previous_file_in_set(),
                    ActivePanel::Jobs => self.select_previous_job(),
                }
            }
            KeyCode::Down => {
                match self.state.active_panel {
                    ActivePanel::Sets => self.select_next_set(),
                    ActivePanel::Files => self.select_next_file_in_set(),
                    ActivePanel::Jobs => self.select_next_job(),
                }
            }
            KeyCode::Left => {
                self.state.active_panel = ActivePanel::Sets;
            }
            KeyCode::Right => {
                self.focus_files_panel();
            }
            KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace => {
                if self.state.active_panel == ActivePanel::Jobs {
                    self.remove_selected_job();
                }
            }
            _ => {}
        }
    }

    fn handle_settings_mode_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Normal;
                if self.state.rescan_needed {
                    self.state.status_message = Some("Exited settings. Ctrl+R to apply algo/parallel changes.".to_string());
                }
                if self.state.sort_settings_changed {
                    self.apply_sort_settings(); // Apply sort changes immediately on exiting settings
                    self.state.status_message = Some(self.state.status_message.clone().map_or("".to_string(), |s| s + " ") + "Sort settings applied.");
                } 
                if !self.state.rescan_needed && !self.state.sort_settings_changed { // access sort_settings_changed *after* it might have been reset by apply_sort_settings
                     self.state.status_message = Some("Exited settings mode.".to_string());
                }
                self.state.sort_settings_changed = false; // Reset flag after processing
            }
            KeyCode::Up => {
                self.state.selected_setting_category_index = self.state.selected_setting_category_index.saturating_sub(1);
            }
            KeyCode::Down => {
                self.state.selected_setting_category_index = (self.state.selected_setting_category_index + 1).min(4); // Max index is 4 now
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
            // Sort Criterion Keys (f, z, c, m, p) - for FileName, FileSize, CreatedAt, ModifiedAt, PathLength
            KeyCode::Char('f') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::FileName;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: File Name (apply on exit)".to_string());
            }
            KeyCode::Char('z') if self.state.selected_setting_category_index == 3 => { // z for siZe
                self.state.current_sort_criterion = SortCriterion::FileSize;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: File Size (apply on exit)".to_string());
            }
            KeyCode::Char('c') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::CreatedAt;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: Created Date (apply on exit)".to_string());
            }
            KeyCode::Char('m') if self.state.selected_setting_category_index == 3 => { // m for modified
                self.state.current_sort_criterion = SortCriterion::ModifiedAt;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: Modified Date (apply on exit)".to_string());
            }
            KeyCode::Char('p') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::PathLength;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: Path Length (apply on exit)".to_string());
            }
            // Sort Order Keys (a, d) - for Ascending, Descending
            KeyCode::Char('a') if self.state.selected_setting_category_index == 4 => {
                self.state.current_sort_order = SortOrder::Ascending;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort Order: Ascending (apply on exit)".to_string());
            }
            KeyCode::Char('d') if self.state.selected_setting_category_index == 4 => {
                self.state.current_sort_order = SortOrder::Descending;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort Order: Descending (apply on exit)".to_string());
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
        if !self.state.display_list.is_empty() {
            self.state.active_panel = ActivePanel::Files;
        }
    }

    fn select_next_set(&mut self) {
        if !self.state.display_list.is_empty() {
            self.state.selected_display_list_index = 
                (self.state.selected_display_list_index + 1) % self.state.display_list.len();
            self.state.selected_file_index_in_set = 0;
        }
    }

    fn select_previous_set(&mut self) {
        if !self.state.display_list.is_empty() {
            if self.state.selected_display_list_index > 0 {
                self.state.selected_display_list_index -= 1;
            } else {
                self.state.selected_display_list_index = self.state.display_list.len() - 1;
            }
            self.state.selected_file_index_in_set = 0;
        }
    }

    fn select_next_file_in_set(&mut self) {
        if let Some(set) = self.current_selected_set_from_display_list() {
            if !set.files.is_empty() {
                self.state.selected_file_index_in_set = 
                    (self.state.selected_file_index_in_set + 1) % set.files.len();
            }
        }
    }

    fn select_previous_file_in_set(&mut self) {
        if let Some(set) = self.current_selected_set_from_display_list() {
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
        let file_index_in_set = self.state.selected_file_index_in_set;
        let mut _status_update: Option<String> = None;
        let mut jobs_to_add: Vec<Job> = Vec::new();
        let mut paths_in_set_to_update_jobs_for: Vec<PathBuf> = Vec::new();
        let mut file_to_keep_path_option: Option<PathBuf> = None;

        if let Some(current_duplicate_set_ref) = self.current_selected_set_from_display_list() {
            if let Some(file_to_keep_cloned) = current_duplicate_set_ref.files.get(file_index_in_set).cloned() {
                log::info!("User designated {:?} as to be KEPT.", file_to_keep_cloned.path);
                _status_update = Some(format!("Marked {} to be KEPT.", file_to_keep_cloned.path.file_name().unwrap_or_default().to_string_lossy()));

                file_to_keep_path_option = Some(file_to_keep_cloned.path.clone());
                jobs_to_add.push(Job { action: ActionType::Keep, file_info: file_to_keep_cloned.clone() });
                
                paths_in_set_to_update_jobs_for = current_duplicate_set_ref.files.iter().map(|f| f.path.clone()).collect();

                for file_in_set in &current_duplicate_set_ref.files {
                    if file_in_set.path != file_to_keep_cloned.path {
                        // Check if already ignored before deciding to mark for delete
                        let is_ignored = self.state.jobs.iter().any(|job|
                            job.file_info.path == file_in_set.path && job.action == ActionType::Ignore
                        );
                        if !is_ignored {
                            jobs_to_add.push(Job { action: ActionType::Delete, file_info: file_in_set.clone() });
                            log::debug!("Auto-marking {:?} for DELETE as another file in set is kept.", file_in_set.path);
                        }
                    }
                }
            } else {
                _status_update = Some("No file selected in set, or set is empty.".to_string());
            }
        } else {
            _status_update = Some("No duplicate set selected (or a folder is selected).".to_string());
        }

        // Now, perform mutations to self.state *after* borrows from current_selected_set_from_display_list are dropped
        if let Some(msg) = _status_update {
            self.state.status_message = Some(msg);
        }

        if let Some(_kept_path) = file_to_keep_path_option.take() {
            // Remove all existing jobs for any file in this specific set first
            // This is important to handle re-marking a different file as kept, or changing mind.
            if !paths_in_set_to_update_jobs_for.is_empty() {
                 self.state.jobs.retain(|job| !paths_in_set_to_update_jobs_for.contains(&job.file_info.path));
            }
            // Then add the new jobs decided above
            self.state.jobs.extend(jobs_to_add);
        } else if !jobs_to_add.is_empty(){
             // This case might happen if only a delete was added without a keep (e.g. if logic changes)
             // For now, if no file_to_keep was identified, we only update status.
             // If jobs_to_add contains items but file_to_keep_path_option is None, it implies an issue or an edge case not fully handled.
             // However, the current logic ensures jobs_to_add is only populated if file_to_keep is found.
        }
    }
    
    fn mark_set_for_deletion(&mut self) {
        if let Some(selected_set_to_action) = self.current_selected_set_from_display_list().cloned() { // Use the renamed method
            if selected_set_to_action.files.len() < 2 {
                self.state.status_message = Some("Set has less than 2 files, no action taken.".to_string());
                return;
            }

            match file_utils::determine_action_targets(&selected_set_to_action, self.state.default_selection_strategy) {
                Ok((kept_file, files_to_delete)) => {
                    let kept_file_path = kept_file.path.clone();
                    let mut files_marked_for_delete = 0;

                    // First, remove any existing jobs for files in this set
                    self.state.jobs.retain(|job| {
                        !selected_set_to_action.files.iter().any(|f_in_set| f_in_set.path == job.file_info.path)
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
        if self.state.display_list.is_empty() {
            self.state.selected_display_list_index = 0;
            self.state.selected_file_index_in_set = 0;
            return;
        }
        if self.state.selected_display_list_index >= self.state.display_list.len() {
            self.state.selected_display_list_index = self.state.display_list.len().saturating_sub(1);
        }

        if let Some(selected_item) = self.state.display_list.get(self.state.selected_display_list_index) {
            match selected_item {
                DisplayListItem::SetEntry { original_group_index, original_set_index_in_group, .. } => {
                    if let Some(current_set) = self.state.grouped_data.get(*original_group_index)
                                                .and_then(|group| group.sets.get(*original_set_index_in_group)) {
                        if current_set.files.is_empty() {
                            self.state.selected_file_index_in_set = 0;
                        } else if self.state.selected_file_index_in_set >= current_set.files.len() {
                            self.state.selected_file_index_in_set = current_set.files.len().saturating_sub(1);
                        }
                    } else {
                        self.state.selected_file_index_in_set = 0; // Should not happen if display_list is sync with grouped_data
                    }
                }
                DisplayListItem::Folder { .. } => {
                    self.state.selected_file_index_in_set = 0; // No files to select when a folder is selected
                }
            }
        } else {
             // display_list is empty or index out of bounds (should be caught by earlier check)
            self.state.selected_file_index_in_set = 0;
        }

        if self.state.jobs.is_empty() {
            self.state.selected_job_index = 0;
        } else if self.state.selected_job_index >= self.state.jobs.len() {
            self.state.selected_job_index = self.state.jobs.len().saturating_sub(1);
        }
    }

    // Gets the actual DuplicateSet if a SetEntry is selected in the display list
    pub fn current_selected_set_from_display_list(&self) -> Option<&DuplicateSet> {
        if let Some(selected_item) = self.state.display_list.get(self.state.selected_display_list_index) {
            match selected_item {
                DisplayListItem::SetEntry { original_group_index, original_set_index_in_group, .. } => {
                    self.state.grouped_data.get(*original_group_index)
                        .and_then(|group| group.sets.get(*original_set_index_in_group))
                }
                DisplayListItem::Folder { .. } => None, // No specific set if a folder is selected
            }
        } else {
            None
        }
    }

    // Current selected file in the middle panel, uses current_selected_set_from_display_list
    pub fn current_selected_file(&self) -> Option<&FileInfo> {
        self.current_selected_set_from_display_list()
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

    fn handle_help_mode_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Normal;
                self.state.status_message = Some("Exited help screen.".to_string());
            }
            _ => {} // Other keys do nothing in help mode
        }
    }

    fn rebuild_display_list(&mut self) {
        self.state.display_list = App::build_display_list_from_grouped_data(&self.state.grouped_data);
        self.validate_selection_indices(); // Ensure selection is still valid
    }

    fn apply_sort_settings(&mut self) {
        log::info!("Applying sort settings: {:?} {:?}", self.state.current_sort_criterion, self.state.current_sort_order);
        for group in &mut self.state.grouped_data {
            for set in &mut group.sets {
                // Use the utility from file_utils, assuming it's public or in the same module 
                // If not, we might need to replicate or expose it.
                // For now, assuming file_utils::sort_file_infos is accessible.
                // It needs to be `pub(crate)` or public in `file_utils`.
                file_utils::sort_file_infos(&mut set.files, self.state.current_sort_criterion, self.state.current_sort_order);
            }
        }
        self.rebuild_display_list(); // This will also validate selections
        self.state.sort_settings_changed = false; // Reset flag
        self.state.status_message = Some("Sort settings applied to current view.".to_string());
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

fn format_file_size(size: u64, raw_sizes: bool) -> String {
    if raw_sizes {
        format!("{} bytes", size)
    } else {
        format_size(size, DECIMAL)
    }
}

fn ui(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Length(3),  // Status
            Constraint::Min(0),     // Main content
            Constraint::Length(1),  // Help bar
        ])
        .split(frame.size());

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
        let mut sort_criterion_style = Style::default();
        let mut sort_order_style = Style::default();

        match app.state.selected_setting_category_index {
            0 => strategy_style = strategy_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            1 => algo_style = algo_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            2 => parallel_style = parallel_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            3 => sort_criterion_style = sort_criterion_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            4 => sort_order_style = sort_order_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            _ => {}
        }

        let settings_text = vec![
            Line::from(Span::styled(format!("1. File Selection Strategy: {:?}", app.state.default_selection_strategy), strategy_style)),
            Line::from(Span::styled(format!("   (n:newest, o:oldest, s:shortest, l:longest)"), strategy_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("2. Hashing Algorithm: {}", app.state.current_algorithm), algo_style)),
            Line::from(Span::styled(format!("   (m:md5, a:sha256, b:blake3)"), algo_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("3. Parallel Cores: {}", 
                app.state.current_parallel.map_or_else(
                    || format!("Auto ({} cores)", num_cpus::get()), 
                    |c| c.to_string()
                )
            ), parallel_style)),
            Line::from(Span::styled(format!("   (0 for auto, 1-N, +/-, requires rescan)"), parallel_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("4. Sort Files By: {:?}", app.state.current_sort_criterion), sort_criterion_style)),
            Line::from(Span::styled(format!("   (f:name, z:size, c:created, m:modified, p:path length)"), sort_criterion_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("5. Sort Order: {:?}", app.state.current_sort_order), sort_order_style)),
            Line::from(Span::styled(format!("   (a:ascending, d:descending)"), sort_order_style)),
            Line::from(Span::raw("")),
            Line::from(Span::raw(if app.state.rescan_needed && app.state.sort_settings_changed {
                "[!] Algorithm/Parallelism and Sort settings changed. Ctrl+R to rescan, Sort applied on Esc."
            } else if app.state.rescan_needed {
                "[!] Algorithm/Parallelism settings changed. Press Ctrl+R to rescan."
            } else if app.state.sort_settings_changed {
                "[!] Sort settings changed. Applied on exiting settings (Esc)."
            } else {
                "No pending setting changes."
            })),
        ];
        let settings_paragraph = Paragraph::new(settings_text)
            .block(Block::default().borders(Borders::ALL).title("Options"))
            .wrap(Wrap { trim: true });
        frame.render_widget(settings_paragraph, chunks[1]);

        let hint = Paragraph::new("Esc: Exit Settings | Use indicated keys to change values.")
            .alignment(Alignment::Center);
        frame.render_widget(hint, chunks[2]);

    } else if app.state.input_mode == InputMode::Help {
        let help_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(0),   // Content
                Constraint::Length(1), // Footer
            ])
            .split(frame.size());

        let title = Paragraph::new("--- Dedup TUI Help ---")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Help Screen"));
        frame.render_widget(title, help_chunks[0]);

        let help_text_lines = vec![
            Line::from(Span::styled("General Navigation:", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  q          : Quit application"),
            Line::from("  Tab        : Cycle focus between Panels (Sets/Folders -> Files -> Jobs)"),
            Line::from("  h          : Show this Help screen (Esc to close)"),
            Line::from("  Ctrl+R     : Trigger a rescan with current settings"),
            Line::from("  Ctrl+S     : Open Settings menu (Esc to close)"),
            Line::from("  Ctrl+E     : Execute all pending jobs"),
            Line::from(""),
            Line::from(Span::styled("Sets/Folders Panel (Left):", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  Up/k       : Select previous folder/set"),
            Line::from("  Down/j     : Select next folder/set"),
            Line::from("  Enter/l    : Focus Files panel for selected set / Expand/Collapse folder (TODO)"),
            Line::from("  d          : Mark all but one file (per strategy) in selected set for deletion"),
            // Line::from("  Ctrl+A : Select all files in all sets for action (TODO)"),
            // Line::from("  /        : Filter sets by regex (TODO)"),
            Line::from(""),
            Line::from(Span::styled("Files Panel (Middle):", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  Up/k       : Select previous file in set"),
            Line::from("  Down/j     : Select next file in set"),
            Line::from("  Left/h     : Focus Sets/Folders panel"),
            Line::from("  s          : Mark selected file to be KEPT (others in set marked for DELETE)"),
            Line::from("  d          : Mark selected file for DELETE"),
            Line::from("  c          : Mark selected file for COPY (prompts for destination)"),
            Line::from("  i          : Mark selected file to be IGNORED (won't be deleted/moved/copied)"),
            Line::from(""),
            Line::from(Span::styled("Jobs Panel (Right):", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  Up/k       : Select previous job"),
            Line::from("  Down/j     : Select next job"),
            Line::from("  x/Del/Bsp  : Remove selected job"),
            Line::from(""),
            Line::from(Span::styled("Settings Menu (Ctrl+S to access):", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  Up/Down    : Navigate setting categories"),
            Line::from("  Strategy   : n (Newest), o (Oldest), s (Shortest Path), l (Longest Path)"),
            Line::from("  Algorithm  : m (md5), a (sha256), b (blake3) - requires rescan"),
            Line::from("  Parallelism: 0 (Auto), 1-9, + (Increment), - (Decrement) - requires rescan"),
            Line::from("  Sorting    : (TODO: Sort By, Sort Order)"),
            Line::from("  Esc        : Exit settings menu"),
            Line::from(""),
            Line::from(Span::styled("Input Prompts (e.g., Copy Destination):", Style::default().add_modifier(Modifier::BOLD))),
            Line::from("  Enter      : Confirm input"),
            Line::from("  Esc        : Cancel input"),
        ];

        let help_paragraph = Paragraph::new(help_text_lines)
            .block(Block::default().borders(Borders::ALL).title("Keybindings"))
            .wrap(Wrap { trim: true });
        frame.render_widget(help_paragraph, help_chunks[1]);

        let footer = Paragraph::new("Press 'Esc' to close Help.")
            .alignment(Alignment::Center);
        frame.render_widget(footer, help_chunks[2]);

    } else {
        // Main UI (3 panels + status bar)
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(70),
                Constraint::Percentage(30),
            ])
            .split(chunks[2]);

        // Helper to create a block with a title and border, highlighting if active
        let create_block = |title_string: String, is_active: bool| {
            let base_style = if is_active { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::White) };
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title_string, base_style))
                .border_style(base_style)
        };

        // Left Panel: Duplicate Sets (actually folders and sets)
        let sets_panel_title_string = format!("Parent Folders / Duplicate Sets ({}/{}) (Tab to navigate)", 
            app.state.selected_display_list_index.saturating_add(1).min(app.state.display_list.len()), 
            app.state.display_list.len()
        );
        let sets_block = create_block(sets_panel_title_string, app.state.active_panel == ActivePanel::Sets && app.state.input_mode == InputMode::Normal);
        
        let list_items: Vec<ListItem> = app.state.display_list.iter().map(|item| {
            match item {
                DisplayListItem::Folder { path, is_expanded, set_count, .. } => {
                    let prefix = if *is_expanded { "[-]" } else { "[+]" };
                    ListItem::new(Line::from(Span::styled(
                        format!("{} {} ({} sets)", prefix, path.display(), set_count),
                        Style::default().add_modifier(Modifier::BOLD)
                    )))
                }
                DisplayListItem::SetEntry { set_hash_preview, set_total_size, file_count_in_set, indent, .. } => {
                    let indent_str = if *indent { "  " } else { "" };
                    ListItem::new(Line::from(Span::styled(
                        format!("{}Hash: {}... ({} files, {})", 
                            indent_str,
                            set_hash_preview, 
                            file_count_in_set, 
                            format_file_size(*set_total_size, app.cli_config.raw_sizes)
                        ),
                        Style::default()
                    )))
                }
            }
        }).collect();

        let sets_list = List::new(list_items)
            .block(sets_block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::Blue))
            .highlight_symbol(">> ");
        let mut sets_list_state = ListState::default();
        if !app.state.display_list.is_empty() {
            sets_list_state.select(Some(app.state.selected_display_list_index));
        }
        frame.render_stateful_widget(sets_list, main_chunks[0], &mut sets_list_state);

        // Middle Panel: Files in Selected Set
        let (files_panel_title_string, file_items) = 
            if let Some(selected_set) = app.current_selected_set_from_display_list() {
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
        if app.current_selected_set_from_display_list().map_or(false, |s| !s.files.is_empty()) {
            files_list_state.select(Some(app.state.selected_file_index_in_set));
        }
        frame.render_stateful_widget(files_list, main_chunks[0], &mut files_list_state);

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
        frame.render_stateful_widget(jobs_list_widget, main_chunks[1], &mut jobs_list_state);

        // Status Bar / Input Area
        match app.state.input_mode {
            InputMode::Normal => {
                let status_text = app.state.status_message.as_deref().unwrap_or("q:quit | Tab:cycle | Arrows/jk:nav | s:keep d:del c:copy i:ign | Ctrl+E:exec | Ctrl+R:rescan | Ctrl+S:settings | x:del job");
                let status_bar = Paragraph::new(status_text)
                    .style(Style::default().fg(Color::LightCyan))
                    .alignment(Alignment::Left);
                frame.render_widget(status_bar, chunks[3]);
            }
            InputMode::CopyDestination => { 
                let input_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Length(1)]).split(chunks[3]);
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
            InputMode::Help => {
                // The Help mode has its own full-screen UI, so no specific status bar here.
                // The hints are part of the help_paragraph and footer Paragraph already rendered.
            }
        }

        // Draw help bar at the bottom
        let help = "h: Help | ↑/↓: Navigate | Space: Toggle | a: Select All | q: Quit";
        let help = Paragraph::new(help)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(help, chunks[3]);
    }
}

// TODO: Define Job struct and ActionType enum for the right panel
// enum ActionType { Delete, Copy, Move }
// struct Job { action: ActionType, file_info: FileInfo, destination: Option<PathBuf> } 