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
use num_cpus; // For displaying actual core count in auto mode
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::collections::HashMap; // For grouping
use std::io::{stdout, Stdout};
use std::path::{Path, PathBuf}; // Ensure Path is imported here
use std::sync::mpsc as std_mpsc; // Alias to avoid conflict if crate::mpsc is used elsewhere
use std::thread as std_thread; // Alias for clarity
use std::time::{Duration, Instant};
use tui_input::backend::crossterm::EventHandler; // For tui-input
use tui_input::Input;

use crate::file_utils::{
    self, delete_files, move_files, DuplicateSet, FileInfo, SelectionStrategy, SortCriterion,
    SortOrder,
};
use crate::Cli; // Added SortCriterion, SortOrder

// Application state
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] // Added PartialEq, Eq
pub enum ActionType {
    Keep, // Implicit action for the one file not chosen for delete/move
    Delete,
    Move(PathBuf), // Target directory for move
    Copy(PathBuf), // Target directory for copy
    Ignore,        // New action type
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
    Help,     // New mode for help screen
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
    pub status_message: Option<String>,                // For feedback
    pub input_mode: InputMode,
    pub current_input: Input,                 // Using tui-input crate
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
    pub current_sort_criterion: SortCriterion,  // New for sorting
    pub current_sort_order: SortOrder,          // New for sorting
    pub sort_settings_changed: bool,            // Flag if sorting needs re-application

    // Media deduplication options
    pub media_mode: bool,
    pub media_resolution: String,
    pub media_formats: Vec<String>,
    pub media_similarity: u32,

    pub log_messages: Vec<String>,  // For operation output
    pub log_scroll: usize,          // For scrolling the log
    pub log_focus: bool,            // Whether log area is focused
    pub log_filter: Option<String>, // For filtering (stub for now)

    pub is_processing_jobs: bool,
    pub job_processing_message: String,
    pub job_progress: (usize, usize), // (done, total)

    pub dry_run: bool, // Indicates if actions should be performed in dry run mode
}

// Channel for messages from scan thread to TUI thread
#[derive(Debug)]
pub enum ScanMessage {
    StatusUpdate(u8, String), // Stage number (1-3) + message
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
    cli_config: Cli,                                // Store the initial CLI config
}

impl App {
    pub fn new(cli_args: &Cli) -> Self {
        let strategy = SelectionStrategy::from_str(&cli_args.mode)
            .unwrap_or(SelectionStrategy::NewestModified);
        let initial_status = "Preparing to scan for duplicates...";

        let app_state = AppState {
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
            loading_message: initial_status.to_string(),
            current_algorithm: cli_args.algorithm.clone(),
            current_parallel: cli_args.parallel,
            rescan_needed: false,
            selected_setting_category_index: 0,
            current_sort_criterion: cli_args.sort_by, // Initialize from Cli
            current_sort_order: cli_args.sort_order,  // Initialize from Cli
            sort_settings_changed: false,
            media_mode: cli_args.media_mode,
            media_resolution: cli_args.media_resolution.clone(),
            media_formats: cli_args.media_formats.clone(),
            media_similarity: cli_args.media_similarity,
            log_messages: Vec::new(),
            log_scroll: 0,
            log_focus: false,
            log_filter: None,
            is_processing_jobs: false,
            job_processing_message: String::new(),
            job_progress: (0, 0),
            dry_run: cli_args.dry_run, // Initialize from CLI args
        };

        // Always perform async scan for TUI
        log::info!(
            "Initializing TUI with directory: {:?}",
            cli_args.directories[0]
        );
        let (tx, rx) = std_mpsc::channel::<ScanMessage>();

        // Send an immediate status update to show we're properly initialized
        tx.send(ScanMessage::StatusUpdate(
            1,
            format!("Starting scan of {}...", cli_args.directories[0].display()),
        ))
        .unwrap_or_else(|e| log::error!("Failed to send initial status update: {}", e));

        let mut current_cli_for_scan = cli_args.clone();
        current_cli_for_scan.algorithm = app_state.current_algorithm.clone();
        current_cli_for_scan.parallel = app_state.current_parallel;
        current_cli_for_scan.sort_by = app_state.current_sort_criterion;
        current_cli_for_scan.sort_order = app_state.current_sort_order;

        log::info!(
            "Starting scan thread with algorithm={}, parallel={:?}",
            current_cli_for_scan.algorithm,
            current_cli_for_scan.parallel
        );

        let thread_tx = tx.clone();
        let scan_thread = std_thread::spawn(move || {
            log::info!("[ScanThread] Starting initial duplicate scan...");
            thread_tx
                .send(ScanMessage::StatusUpdate(
                    1,
                    "Scan thread initialized, starting file scan...".to_string(),
                ))
                .unwrap_or_else(|e| {
                    log::error!("[ScanThread] Failed to send initialization message: {}", e)
                });

            match file_utils::find_duplicate_files_with_progress(
                &current_cli_for_scan,
                thread_tx.clone(),
            ) {
                Ok(raw_sets) => {
                    log::info!(
                        "[ScanThread] Scan completed successfully with {} sets",
                        raw_sets.len()
                    );
                    if thread_tx
                        .send(ScanMessage::Completed(Ok(raw_sets)))
                        .is_err()
                    {
                        log::error!("[ScanThread] Failed to send completion message to TUI.");
                    }
                }
                Err(e) => {
                    log::error!("[ScanThread] Scan failed with error: {}", e);
                    if thread_tx.send(ScanMessage::Error(e.to_string())).is_err() {
                        log::error!("[ScanThread] Failed to send error message to TUI.");
                    }
                }
            }
            log::info!("[ScanThread] Initial scan finished.");
        });

        // Wrap the thread in Some() with error handling
        let scan_join_handle = match scan_thread.thread().id() {
            id => {
                log::info!("Scan thread started with ID: {:?}", id);
                Some(scan_thread)
            }
        };

        Self {
            state: app_state,
            should_quit: false,
            scan_thread_join_handle: scan_join_handle,
            scan_rx: Some(rx),
            scan_tx: Some(tx),
            cli_config: cli_args.clone(),
        }
    }

    fn process_raw_sets_into_grouped_view(
        sets: Vec<DuplicateSet>,
        default_expanded: bool,
    ) -> (Vec<ParentFolderGroup>, Vec<DisplayListItem>) {
        let mut parent_map: HashMap<PathBuf, Vec<DuplicateSet>> = HashMap::new();
        for set in sets {
            if let Some(first_file) = set.files.first() {
                let parent = first_file
                    .path
                    .parent()
                    .unwrap_or_else(|| Path::new("/"))
                    .to_path_buf();
                parent_map.entry(parent).or_default().push(set);
            }
        }

        let mut grouped_data: Vec<ParentFolderGroup> = parent_map
            .into_iter()
            .map(|(path, sets_in_group)| ParentFolderGroup {
                path,
                sets: sets_in_group,
                is_expanded: default_expanded,
            })
            .collect();

        grouped_data.sort_by(|a, b| a.path.cmp(&b.path));
        for group in &mut grouped_data {
            group.sets.sort_by(|a, b| a.hash.cmp(&b.hash)); // Ensure consistent order of sets within a folder
        }

        let display_list = App::build_display_list_from_grouped_data(&grouped_data);
        (grouped_data, display_list)
    }

    fn build_display_list_from_grouped_data(
        grouped_data: &[ParentFolderGroup],
    ) -> Vec<DisplayListItem> {
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
        self.state.loading_message = "⏳ [0/3] Preparing for rescan...".to_string();
        self.state.status_message = Some("Starting rescan...".to_string());
        self.state.rescan_needed = false; // Reset flag as we are acting on it

        let mut current_cli_for_scan = self.cli_config.clone(); // Use stored cli_config
        current_cli_for_scan.algorithm = self.state.current_algorithm.clone();
        current_cli_for_scan.parallel = self.state.current_parallel;
        current_cli_for_scan.sort_by = self.state.current_sort_criterion;
        current_cli_for_scan.sort_order = self.state.current_sort_order;
        // Always enable progress for TUI mode
        current_cli_for_scan.progress = true;
        current_cli_for_scan.progress_tui = true;

        // Apply media deduplication options
        current_cli_for_scan.media_mode = self.state.media_mode;
        current_cli_for_scan.media_resolution = self.state.media_resolution.clone();
        current_cli_for_scan.media_formats = self.state.media_formats.clone();
        current_cli_for_scan.media_similarity = self.state.media_similarity;

        // If media mode is enabled, set up the media_dedup_options
        if current_cli_for_scan.media_mode {
            // Clear any existing options first
            current_cli_for_scan.media_dedup_options =
                crate::media_dedup::MediaDedupOptions::default();

            // Apply settings to media_dedup_options
            crate::media_dedup::add_media_options_to_cli(
                &mut current_cli_for_scan.media_dedup_options,
                self.state.media_mode,
                &self.state.media_resolution,
                &self.state.media_formats,
                self.state.media_similarity,
            );
        }

        // Note: We always use progress for TUI internal scans regardless of initial cli.progress
        // find_duplicate_files_with_progress requires a tx channel.
        // Ensure scan_tx is Some.

        // Create a fresh channel, assign the receiver to our app's scan_rx
        let (tx, rx) = std_mpsc::channel::<ScanMessage>();
        self.scan_rx = Some(rx);

        // Send an initial status to note the rescan
        tx.send(ScanMessage::StatusUpdate(
            1,
            "Starting new scan...".to_string(),
        ))
        .unwrap_or_else(|e| log::error!("Failed to send initial rescan status: {}", e));

        // Create the scan thread
        let thread_tx = tx.clone();
        let scan_thread = std_thread::spawn(move || {
            log::info!("[ScanThread] Starting rescan...");
            match file_utils::find_duplicate_files_with_progress(
                &current_cli_for_scan,
                thread_tx.clone(),
            ) {
                Ok(raw_sets) => {
                    log::info!(
                        "[ScanThread] Rescan completed successfully with {} sets",
                        raw_sets.len()
                    );
                    if thread_tx
                        .send(ScanMessage::Completed(Ok(raw_sets)))
                        .is_err()
                    {
                        log::error!("[ScanThread] Failed to send rescan completion to TUI.");
                    }
                }
                Err(e) => {
                    log::error!("[ScanThread] Rescan failed with error: {}", e);
                    if thread_tx.send(ScanMessage::Error(e.to_string())).is_err() {
                        log::error!("[ScanThread] Failed to send rescan error to TUI.");
                    }
                }
            }
            log::info!("[ScanThread] Rescan finished.");
        });

        let scan_join_handle = match scan_thread.thread().id() {
            id => {
                log::info!("Rescan thread started with ID: {:?}", id);
                Some(scan_thread)
            }
        };

        self.scan_thread_join_handle = scan_join_handle;
        self.scan_tx = Some(tx);
    }

    // Method to handle messages from the scan thread
    pub fn handle_scan_messages(&mut self) {
        if let Some(ref rx) = self.scan_rx {
            match rx.try_recv() {
                Ok(message) => {
                    match message {
                        ScanMessage::StatusUpdate(stage, msg) => {
                            // Format the stage indicator for display
                            let stage_prefix = match stage {
                                0 => "⏳ [0/3] ", // Pre-scan stage
                                1 => "📁 [1/3] ",
                                2 => "🔍 [2/3] ",
                                3 => "🔄 [3/3] ",
                                _ => "",
                            };

                            self.state.loading_message = format!("{}{}", stage_prefix, msg);
                            log::debug!("Updated loading message: {}", self.state.loading_message);
                        }
                        ScanMessage::Completed(result) => {
                            match result {
                                Ok(sets) => {
                                    log::info!("Scan completed with {} sets", sets.len());
                                    self.state.is_loading = false;

                                    // Process the raw sets into our grouped view
                                    let (grouped_data, display_list) =
                                        App::process_raw_sets_into_grouped_view(sets, true);
                                    self.state.grouped_data = grouped_data;
                                    self.state.display_list = display_list;

                                    // Apply current sort settings to the loaded data
                                    self.apply_sort_settings();

                                    self.state.status_message = Some(format!(
                                        "Scan complete! Found {} duplicate sets.",
                                        self.state
                                            .grouped_data
                                            .iter()
                                            .map(|g| g.sets.len())
                                            .sum::<usize>()
                                    ));
                                }
                                Err(e) => {
                                    log::error!("Scan completed with error: {}", e);
                                    self.state.is_loading = false;
                                    self.state.status_message = Some(format!("Scan failed: {}", e));
                                }
                            }
                        }
                        ScanMessage::Error(err) => {
                            log::error!("Scan error: {}", err);
                            self.state.is_loading = false;
                            self.state.status_message = Some(format!("Scan error: {}", err));
                        }
                    }
                }
                Err(std_mpsc::TryRecvError::Empty) => {
                    // No messages available, perfectly normal.
                }
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    // Channel disconnected. This could happen if the scan thread finishes.
                    log::warn!("Scan thread channel disconnected.");
                    if self.state.is_loading {
                        // If still in loading state, this is an error.
                        self.state.is_loading = false;
                        self.state.status_message =
                            Some("Scan thread disconnected unexpectedly.".to_string());
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
            KeyCode::Char('q') | KeyCode::Char('c')
                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.should_quit = true;
            }
            KeyCode::Char('h') => {
                self.state.input_mode = InputMode::Help;
                self.state.status_message = Some("Displaying Help. Esc to exit.".to_string());
            }
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // Toggle dry run mode
                self.state.dry_run = !self.state.dry_run;
                let status = if self.state.dry_run {
                    "Dry run mode ENABLED - No actual changes will be made"
                } else {
                    "Dry run mode DISABLED - Actions will perform actual changes"
                };
                self.state.status_message = Some(status.to_string());
                self.state.log_messages.push(status.to_string());

                // Add more information about dry run mode when enabled
                if self.state.dry_run {
                    self.state.log_messages.push(
                        "In dry run mode, all operations are simulated and logged but no actual changes are made to files.".to_string()
                    );
                    if !self.state.jobs.is_empty() {
                        self.state.log_messages.push(
                            format!("Current job queue contains {} operations that will be simulated when executed.",
                                   self.state.jobs.len())
                        );
                    }
                }

                log::info!("{}", status);
            }
            KeyCode::Char('a') => {
                // Global toggle between deleting ALL files or KEEPING all (no explicit jobs).
                // To avoid huge memory spikes we only create Delete jobs when needed and
                // never generate explicit Keep jobs (no job = Keep).

                // 1. Count total files across all duplicate sets.
                let total_files: usize = self
                    .state
                    .grouped_data
                    .iter()
                    .map(|g| g.sets.iter().map(|s| s.files.len()).sum::<usize>())
                    .sum();

                // 2. Count current Delete jobs.
                let current_delete_jobs = self
                    .state
                    .jobs
                    .iter()
                    .filter(|j| matches!(j.action, ActionType::Delete))
                    .count();

                let currently_all_deleted = current_delete_jobs == total_files && total_files > 0;

                if currently_all_deleted {
                    // Toggle to KEEP all: simply clear the job list.
                    self.state.jobs.clear();
                    self.state.status_message =
                        Some("All delete jobs cleared. All files kept.".to_string());
                    self.state
                        .log_messages
                        .push("Toggled: KEEP all files (cleared delete jobs)".to_string());
                } else {
                    // Toggle to DELETE all: rebuild jobs list with Delete actions for every file.
                    self.state.jobs.clear();

                    // Iterate over grouped_data without cloning large intermediate Vec.
                    for group in &self.state.grouped_data {
                        for set in &group.sets {
                            for file in &set.files {
                                self.state.jobs.push(Job {
                                    action: ActionType::Delete,
                                    file_info: file.clone(),
                                });
                            }
                        }
                    }

                    self.state.status_message =
                        Some(format!("All {} files marked for delete", total_files));
                    self.state
                        .log_messages
                        .push(format!("Toggled: DELETE all {} files", total_files));
                }
            }
            KeyCode::Char('d') => {
                // Mark all files in the selected set or folder for delete
                if let Some(selected) = self
                    .state
                    .display_list
                    .get(self.state.selected_display_list_index)
                {
                    match selected {
                        DisplayListItem::SetEntry {
                            original_group_index,
                            original_set_index_in_group,
                            ..
                        } => {
                            let files_to_process = if let Some(group) =
                                self.state.grouped_data.get(*original_group_index)
                            {
                                group.sets[*original_set_index_in_group].files.clone()
                            } else {
                                Vec::new()
                            };
                            let paths: Vec<_> =
                                files_to_process.iter().map(|f| f.path.clone()).collect();
                            self.state
                                .jobs
                                .retain(|job| !paths.contains(&job.file_info.path));
                            for file in files_to_process {
                                self.state.jobs.push(Job {
                                    action: ActionType::Delete,
                                    file_info: file,
                                });
                            }
                            self.state.status_message =
                                Some("All files in set marked for delete".to_string());
                        }
                        DisplayListItem::Folder { .. } => {
                            // Find the group for this folder
                            let group_index = self.state.display_list
                                [..=self.state.selected_display_list_index]
                                .iter()
                                .filter(|item| matches!(item, DisplayListItem::Folder { .. }))
                                .count()
                                - 1;
                            let files_to_process =
                                if let Some(group) = self.state.grouped_data.get(group_index) {
                                    group
                                        .sets
                                        .iter()
                                        .flat_map(|set| set.files.clone())
                                        .collect::<Vec<_>>()
                                } else {
                                    Vec::new()
                                };
                            let paths: Vec<_> =
                                files_to_process.iter().map(|f| f.path.clone()).collect();
                            self.state
                                .jobs
                                .retain(|job| !paths.contains(&job.file_info.path));
                            for file in files_to_process {
                                self.state.jobs.push(Job {
                                    action: ActionType::Delete,
                                    file_info: file,
                                });
                            }
                            self.state.status_message =
                                Some("All files in folder marked for delete".to_string());
                        }
                    }
                }
            }
            KeyCode::Char('k') => {
                // Mark all files in the selected set or folder for keep
                if let Some(selected) = self
                    .state
                    .display_list
                    .get(self.state.selected_display_list_index)
                {
                    match selected {
                        DisplayListItem::SetEntry {
                            original_group_index,
                            original_set_index_in_group,
                            ..
                        } => {
                            let files_to_process = if let Some(group) =
                                self.state.grouped_data.get(*original_group_index)
                            {
                                group.sets[*original_set_index_in_group].files.clone()
                            } else {
                                Vec::new()
                            };
                            let paths: Vec<_> =
                                files_to_process.iter().map(|f| f.path.clone()).collect();
                            self.state
                                .jobs
                                .retain(|job| !paths.contains(&job.file_info.path));
                            for file in files_to_process {
                                self.state.jobs.push(Job {
                                    action: ActionType::Keep,
                                    file_info: file,
                                });
                            }
                            self.state.status_message =
                                Some("All files in set marked to keep".to_string());
                        }
                        DisplayListItem::Folder { .. } => {
                            // Find the group for this folder
                            let group_index = self.state.display_list
                                [..=self.state.selected_display_list_index]
                                .iter()
                                .filter(|item| matches!(item, DisplayListItem::Folder { .. }))
                                .count()
                                - 1;
                            let files_to_process =
                                if let Some(group) = self.state.grouped_data.get(group_index) {
                                    group
                                        .sets
                                        .iter()
                                        .flat_map(|set| set.files.clone())
                                        .collect::<Vec<_>>()
                                } else {
                                    Vec::new()
                                };
                            let paths: Vec<_> =
                                files_to_process.iter().map(|f| f.path.clone()).collect();
                            self.state
                                .jobs
                                .retain(|job| !paths.contains(&job.file_info.path));
                            for file in files_to_process {
                                self.state.jobs.push(Job {
                                    action: ActionType::Keep,
                                    file_info: file,
                                });
                            }
                            self.state.status_message =
                                Some("All files in folder marked to keep".to_string());
                        }
                    }
                }
            }
            KeyCode::Tab => {
                self.cycle_active_panel();
            }
            KeyCode::Char('e') => {
                let result = self.process_pending_jobs();
                match result {
                    Ok(_) => {
                        self.state
                            .log_messages
                            .push("Executed all pending jobs.".to_string());
                    }
                    Err(e) => {
                        self.state
                            .log_messages
                            .push(format!("Error processing jobs: {}", e));
                    }
                }
            }
            KeyCode::Char('r') => {
                self.trigger_rescan();
            }
            KeyCode::Char('s') => {
                self.state.input_mode = InputMode::Settings;
                self.state.status_message = Some("Entered settings mode. Esc to exit.".to_string());
            }
            KeyCode::Char('i') => {
                self.set_action_for_selected_file(ActionType::Ignore);
            }
            KeyCode::Char('c') => {
                self.initiate_copy_action();
            }
            KeyCode::Up => match self.state.active_panel {
                ActivePanel::Sets => self.select_previous_set(),
                ActivePanel::Files => self.select_previous_file_in_set(),
                ActivePanel::Jobs => self.select_previous_job(),
            },
            KeyCode::Down => match self.state.active_panel {
                ActivePanel::Sets => self.select_next_set(),
                ActivePanel::Files => self.select_next_file_in_set(),
                ActivePanel::Jobs => self.select_next_job(),
            },
            KeyCode::Left => {
                self.state.active_panel = ActivePanel::Sets;
            }
            KeyCode::Right => {
                self.focus_files_panel();
            }
            KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace => {
                // Remove selected job from any panel
                let before = self.state.jobs.len();
                self.remove_selected_job();
                let after = self.state.jobs.len();
                if after < before {
                    self.state.log_messages.push("Job removed.".to_string());
                } else {
                    self.state
                        .log_messages
                        .push("No job selected to remove or jobs list empty.".to_string());
                }
            }
            KeyCode::Char('g') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.log_focus = !self.state.log_focus;
                self.state.status_message = Some(if self.state.log_focus {
                    "Log focus ON (Up/Down/PgUp/PgDn, Ctrl+L: clear, /: filter, Esc: exit log)"
                        .to_string()
                } else {
                    "Log focus OFF".to_string()
                });
            }
            KeyCode::Char('l')
                if key_event.modifiers.contains(KeyModifiers::CONTROL) && self.state.log_focus =>
            {
                self.state.log_messages.clear();
                self.state.log_scroll = 0;
                self.state.status_message = Some("Log cleared.".to_string());
            }
            KeyCode::PageUp if self.state.log_focus => {
                let log_height = 5;
                if self.state.log_scroll >= log_height {
                    self.state.log_scroll -= log_height;
                } else {
                    self.state.log_scroll = 0;
                }
            }
            KeyCode::PageDown if self.state.log_focus => {
                let log_height = 5;
                let max_scroll = self.state.log_messages.len().saturating_sub(log_height);
                if self.state.log_scroll + log_height < max_scroll {
                    self.state.log_scroll += log_height;
                } else {
                    self.state.log_scroll = max_scroll;
                }
            }
            KeyCode::Esc if self.state.log_focus => {
                self.state.log_focus = false;
                self.state.status_message = Some("Exited log focus.".to_string());
            }
            KeyCode::Char('/') if self.state.log_focus => {
                self.state.log_filter = Some(String::new());
                self.state.status_message =
                    Some("Log filter: (type to filter, Esc to clear)".to_string());
            }
            _ => {}
        }
    }

    fn handle_settings_mode_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Normal;
                if self.state.rescan_needed {
                    self.state.status_message =
                        Some("Exited settings. Ctrl+R to apply algo/parallel changes.".to_string());
                }
                if self.state.sort_settings_changed {
                    self.apply_sort_settings(); // Apply sort changes immediately on exiting settings
                    self.state.status_message = Some(
                        self.state
                            .status_message
                            .clone()
                            .map_or("".to_string(), |s| s + " ")
                            + "Sort settings applied.",
                    );
                }
                if !self.state.rescan_needed && !self.state.sort_settings_changed {
                    // access sort_settings_changed *after* it might have been reset by apply_sort_settings
                    self.state.status_message = Some("Exited settings mode.".to_string());
                }
                self.state.sort_settings_changed = false; // Reset flag after processing
            }
            KeyCode::Up => {
                self.state.selected_setting_category_index =
                    self.state.selected_setting_category_index.saturating_sub(1);
            }
            KeyCode::Down => {
                self.state.selected_setting_category_index =
                    (self.state.selected_setting_category_index + 1).min(8); // Max index is 8 now including media options
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
            // Algorithm selection keys (m, a, b, x, g, f, c)
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
            KeyCode::Char('x') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "xxhash".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: xxhash (Rescan needed)".to_string());
            }
            KeyCode::Char('g') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "gxhash".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: gxhash (Rescan needed)".to_string());
            }
            KeyCode::Char('f') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "fnv1a".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: fnv1a (Rescan needed)".to_string());
            }
            KeyCode::Char('c') if self.state.selected_setting_category_index == 1 => {
                self.state.current_algorithm = "crc32".to_string();
                self.state.rescan_needed = true;
                self.state.status_message = Some("Algorithm: crc32 (Rescan needed)".to_string());
            }
            // Parallelism adjustment keys (+, -, 0-9)
            KeyCode::Char('0') if self.state.selected_setting_category_index == 2 => {
                self.state.current_parallel = None; // None signifies auto
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Parallel Cores: Auto (Rescan needed)".to_string());
            }
            KeyCode::Char(c @ '1'..='9') if self.state.selected_setting_category_index == 2 => {
                // Simple single digit for now. Could extend to multi-digit input.
                let cores = c.to_digit(10).map(|d| d as usize);
                if self.state.current_parallel != cores {
                    self.state.current_parallel = cores;
                    self.state.rescan_needed = true;
                    self.state.status_message =
                        Some(format!("Parallel Cores: {} (Rescan needed)", c));
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
                    self.state.status_message =
                        Some(format!("Parallel Cores: {} (Rescan needed)", new_val));
                }
            }
            KeyCode::Char('-') if self.state.selected_setting_category_index == 2 => {
                let current_val = self.state.current_parallel.unwrap_or(1); // If auto (None), treat as 1 for decrement start
                if current_val > 1 {
                    // Minimum 1 core
                    let new_val = current_val - 1;
                    if self.state.current_parallel != Some(new_val) {
                        self.state.current_parallel = Some(new_val);
                        self.state.rescan_needed = true;
                        self.state.status_message =
                            Some(format!("Parallel Cores: {} (Rescan needed)", new_val));
                    }
                } else if current_val == 1 && self.state.current_parallel.is_some() {
                    // Allow going from 1 to Auto (None)
                    self.state.current_parallel = None;
                    self.state.rescan_needed = true;
                    self.state.status_message =
                        Some("Parallel Cores: Auto (Rescan needed)".to_string());
                }
            }
            // Sort Criterion Keys (f, z, c, m, p) - for FileName, FileSize, CreatedAt, ModifiedAt, PathLength
            KeyCode::Char('f') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::FileName;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: File Name (apply on exit)".to_string());
            }
            KeyCode::Char('z') if self.state.selected_setting_category_index == 3 => {
                // z for siZe
                self.state.current_sort_criterion = SortCriterion::FileSize;
                self.state.sort_settings_changed = true;
                self.state.status_message = Some("Sort By: File Size (apply on exit)".to_string());
            }
            KeyCode::Char('c') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::CreatedAt;
                self.state.sort_settings_changed = true;
                self.state.status_message =
                    Some("Sort By: Created Date (apply on exit)".to_string());
            }
            KeyCode::Char('m') if self.state.selected_setting_category_index == 3 => {
                // m for modified
                self.state.current_sort_criterion = SortCriterion::ModifiedAt;
                self.state.sort_settings_changed = true;
                self.state.status_message =
                    Some("Sort By: Modified Date (apply on exit)".to_string());
            }
            KeyCode::Char('p') if self.state.selected_setting_category_index == 3 => {
                self.state.current_sort_criterion = SortCriterion::PathLength;
                self.state.sort_settings_changed = true;
                self.state.status_message =
                    Some("Sort By: Path Length (apply on exit)".to_string());
            }
            // Sort Order Keys (a, d) - for Ascending, Descending
            KeyCode::Char('a') if self.state.selected_setting_category_index == 4 => {
                self.state.current_sort_order = SortOrder::Ascending;
                self.state.sort_settings_changed = true;
                self.state.status_message =
                    Some("Sort Order: Ascending (apply on exit)".to_string());
            }
            KeyCode::Char('d') if self.state.selected_setting_category_index == 4 => {
                self.state.current_sort_order = SortOrder::Descending;
                self.state.sort_settings_changed = true;
                self.state.status_message =
                    Some("Sort Order: Descending (apply on exit)".to_string());
            }
            // Media Deduplication Toggle
            KeyCode::Char('e') if self.state.selected_setting_category_index == 5 => {
                self.state.media_mode = !self.state.media_mode;
                self.state.rescan_needed = true;
                if self.state.media_mode {
                    // Check if ffmpeg is available
                    if crate::media_dedup::is_ffmpeg_available() {
                        self.state.status_message =
                            Some("Media Mode: Enabled (Rescan needed)".to_string());
                    } else {
                        self.state.status_message = Some("Media Mode: Enabled - ffmpeg not found, video processing may be limited (Rescan needed)".to_string());
                        self.state.log_messages.push(
                            "Warning: ffmpeg not found. Video deduplication will be limited."
                                .to_string(),
                        );
                    }
                } else {
                    self.state.status_message =
                        Some("Media Mode: Disabled (Rescan needed)".to_string());
                }
            }
            // Resolution Preference
            KeyCode::Char('h') if self.state.selected_setting_category_index == 6 => {
                self.state.media_resolution = "highest".to_string();
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Resolution Preference: Highest (Rescan needed)".to_string());
            }
            KeyCode::Char('l') if self.state.selected_setting_category_index == 6 => {
                self.state.media_resolution = "lowest".to_string();
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Resolution Preference: Lowest (Rescan needed)".to_string());
            }
            KeyCode::Char('c') if self.state.selected_setting_category_index == 6 => {
                self.state.media_resolution = "1280x720".to_string(); // Default to 720p
                self.state.rescan_needed = true;
                self.state.status_message = Some(
                    "Media Resolution Preference: Custom (1280x720) (Rescan needed)".to_string(),
                );
            }
            // Format Preference
            KeyCode::Char('r') if self.state.selected_setting_category_index == 7 => {
                self.state.media_formats = vec![
                    "raw".to_string(),
                    "png".to_string(),
                    "jpg".to_string(),
                    "mp4".to_string(),
                    "wav".to_string(),
                ];
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Format Preference: RAW > PNG > JPG (Rescan needed)".to_string());
            }
            KeyCode::Char('p') if self.state.selected_setting_category_index == 7 => {
                self.state.media_formats = vec![
                    "png".to_string(),
                    "jpg".to_string(),
                    "raw".to_string(),
                    "mp4".to_string(),
                    "wav".to_string(),
                ];
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Format Preference: PNG > JPG > RAW (Rescan needed)".to_string());
            }
            KeyCode::Char('j') if self.state.selected_setting_category_index == 7 => {
                self.state.media_formats = vec![
                    "jpg".to_string(),
                    "raw".to_string(),
                    "png".to_string(),
                    "mp4".to_string(),
                    "wav".to_string(),
                ];
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Format Preference: JPG > RAW > PNG (Rescan needed)".to_string());
            }
            // Similarity Threshold
            KeyCode::Char('1') if self.state.selected_setting_category_index == 8 => {
                self.state.media_similarity = 95;
                self.state.rescan_needed = true;
                self.state.status_message = Some(
                    "Media Similarity Threshold: 95% (Very strict) (Rescan needed)".to_string(),
                );
            }
            KeyCode::Char('2') if self.state.selected_setting_category_index == 8 => {
                self.state.media_similarity = 90;
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Similarity Threshold: 90% (Default) (Rescan needed)".to_string());
            }
            KeyCode::Char('3') if self.state.selected_setting_category_index == 8 => {
                self.state.media_similarity = 85;
                self.state.rescan_needed = true;
                self.state.status_message =
                    Some("Media Similarity Threshold: 85% (Relaxed) (Rescan needed)".to_string());
            }
            KeyCode::Char('4') if self.state.selected_setting_category_index == 8 => {
                self.state.media_similarity = 75;
                self.state.rescan_needed = true;
                self.state.status_message = Some(
                    "Media Similarity Threshold: 75% (Very relaxed) (Rescan needed)".to_string(),
                );
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
                        self.state.status_message = Some(format!(
                            "Marked {} for copy to {}",
                            file_to_copy.path.display(),
                            dest_path.display()
                        ));
                    } else {
                        self.state.status_message =
                            Some("Copy cancelled: empty destination path.".to_string());
                    }
                } else {
                    self.state.status_message =
                        Some("Copy cancelled: no file selected.".to_string());
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
                self.state
                    .current_input
                    .handle_event(&CEvent::Key(key_event));
            }
        }
    }

    fn initiate_copy_action(&mut self) {
        if let Some(selected_file) = self.current_selected_file().cloned() {
            self.state.file_for_copy_move = Some(selected_file);
            self.state.input_mode = InputMode::CopyDestination;
            self.state.current_input.reset(); // Clear previous input
            self.state.status_message = Some(
                "Enter destination path for copy (Enter to confirm, Esc to cancel):".to_string(),
            );
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
            self.state
                .jobs
                .retain(|job| job.file_info.path != selected_file_info.path);

            // Add the new job
            log::info!(
                "Setting action {:?} for file {:?}",
                action_type,
                selected_file_info.path
            );
            self.state.jobs.push(Job {
                action: action_type.clone(),
                file_info: selected_file_info.clone(),
            });
            self.state.status_message = Some(format!(
                "Marked {} for {:?}.",
                selected_file_info
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                action_type
            ));
        } else {
            self.state.status_message = Some("No file selected to set action.".to_string());
        }
    }

    #[allow(dead_code)]
    fn set_selected_file_as_kept(&mut self) {
        let file_index_in_set = self.state.selected_file_index_in_set;
        let mut _status_update: Option<String> = None;
        let mut jobs_to_add: Vec<Job> = Vec::new();
        let mut paths_in_set_to_update_jobs_for: Vec<PathBuf> = Vec::new();
        let mut file_to_keep_path_option: Option<PathBuf> = None;

        if let Some(current_duplicate_set_ref) = self.current_selected_set_from_display_list() {
            if let Some(file_to_keep_cloned) = current_duplicate_set_ref
                .files
                .get(file_index_in_set)
                .cloned()
            {
                log::info!(
                    "User designated {:?} as to be KEPT.",
                    file_to_keep_cloned.path
                );
                _status_update = Some(format!(
                    "Marked {} to be KEPT.",
                    file_to_keep_cloned
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));

                file_to_keep_path_option = Some(file_to_keep_cloned.path.clone());
                jobs_to_add.push(Job {
                    action: ActionType::Keep,
                    file_info: file_to_keep_cloned.clone(),
                });

                paths_in_set_to_update_jobs_for = current_duplicate_set_ref
                    .files
                    .iter()
                    .map(|f| f.path.clone())
                    .collect();

                for file_in_set in &current_duplicate_set_ref.files {
                    if file_in_set.path != file_to_keep_cloned.path {
                        // Check if already ignored before deciding to mark for delete
                        let is_ignored = self.state.jobs.iter().any(|job| {
                            job.file_info.path == file_in_set.path
                                && job.action == ActionType::Ignore
                        });
                        if !is_ignored {
                            jobs_to_add.push(Job {
                                action: ActionType::Delete,
                                file_info: file_in_set.clone(),
                            });
                            log::debug!(
                                "Auto-marking {:?} for DELETE as another file in set is kept.",
                                file_in_set.path
                            );
                        }
                    }
                }
            } else {
                _status_update = Some("No file selected in set, or set is empty.".to_string());
            }
        } else {
            _status_update =
                Some("No duplicate set selected (or a folder is selected).".to_string());
        }

        // Now, perform mutations to self.state *after* borrows from current_selected_set_from_display_list are dropped
        if let Some(msg) = _status_update {
            self.state.status_message = Some(msg);
        }

        if let Some(_kept_path) = file_to_keep_path_option.take() {
            // Remove all existing jobs for any file in this specific set first
            // This is important to handle re-marking a different file as kept, or changing mind.
            if !paths_in_set_to_update_jobs_for.is_empty() {
                self.state
                    .jobs
                    .retain(|job| !paths_in_set_to_update_jobs_for.contains(&job.file_info.path));
            }
            // Then add the new jobs decided above
            self.state.jobs.extend(jobs_to_add);
        } else if !jobs_to_add.is_empty() {
            // This case might happen if only a delete was added without a keep (e.g. if logic changes)
            // For now, if no file_to_keep was identified, we only update status.
            // If jobs_to_add contains items but file_to_keep_path_option is None, it implies an issue or an edge case not fully handled.
            // However, the current logic ensures jobs_to_add is only populated if file_to_keep is found.
        }
    }

    #[allow(dead_code)]
    fn mark_set_for_deletion(&mut self) {
        if let Some(selected_set_to_action) = self.current_selected_set_from_display_list().cloned()
        {
            // Use the renamed method
            if selected_set_to_action.files.len() < 2 {
                self.state.status_message =
                    Some("Set has less than 2 files, no action taken.".to_string());
                return;
            }

            match file_utils::determine_action_targets(
                &selected_set_to_action,
                self.state.default_selection_strategy,
            ) {
                Ok((kept_file, files_to_delete)) => {
                    let kept_file_path = kept_file.path.clone();
                    let mut files_marked_for_delete = 0;

                    // First, remove any existing jobs for files in this set
                    self.state.jobs.retain(|job| {
                        !selected_set_to_action
                            .files
                            .iter()
                            .any(|f_in_set| f_in_set.path == job.file_info.path)
                    });

                    // Add Keep job for the determined file
                    self.state.jobs.push(Job {
                        action: ActionType::Keep,
                        file_info: kept_file.clone(),
                    });
                    log::info!(
                        "Auto-marking {:?} to KEEP based on strategy {:?}.",
                        kept_file.path,
                        self.state.default_selection_strategy
                    );

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
                    self.state.status_message = Some(format!(
                        "Marked {} files for DELETE, 1 to KEEP in current set.",
                        files_marked_for_delete
                    ));
                }
                Err(e) => {
                    self.state.status_message =
                        Some(format!("Error determining actions for set: {}", e));
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
            self.state.selected_display_list_index =
                self.state.display_list.len().saturating_sub(1);
        }

        if let Some(selected_item) = self
            .state
            .display_list
            .get(self.state.selected_display_list_index)
        {
            match selected_item {
                DisplayListItem::SetEntry {
                    original_group_index,
                    original_set_index_in_group,
                    ..
                } => {
                    if let Some(current_set) = self
                        .state
                        .grouped_data
                        .get(*original_group_index)
                        .and_then(|group| group.sets.get(*original_set_index_in_group))
                    {
                        if current_set.files.is_empty() {
                            self.state.selected_file_index_in_set = 0;
                        } else if self.state.selected_file_index_in_set >= current_set.files.len() {
                            self.state.selected_file_index_in_set =
                                current_set.files.len().saturating_sub(1);
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
        if let Some(selected_item) = self
            .state
            .display_list
            .get(self.state.selected_display_list_index)
        {
            match selected_item {
                DisplayListItem::SetEntry {
                    original_group_index,
                    original_set_index_in_group,
                    ..
                } => self
                    .state
                    .grouped_data
                    .get(*original_group_index)
                    .and_then(|group| group.sets.get(*original_set_index_in_group)),
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
            self.state
                .log_messages
                .push("No jobs to process.".to_string());
            return Ok(());
        }

        // Set the dry_run flag based on app state
        let dry_run_mode = self.state.dry_run;
        if dry_run_mode {
            self.state
                .log_messages
                .push("DRY RUN MODE: Simulating actions without making changes".to_string());
        }

        self.state.is_processing_jobs = true;
        self.state.job_processing_message = if dry_run_mode {
            "Simulating jobs (DRY RUN)..."
        } else {
            "Processing jobs..."
        }
        .to_string();

        let total_jobs = self.state.jobs.len();
        self.state.job_progress = (0, total_jobs);
        let mut success_count = 0;
        let mut fail_count = 0;
        let jobs_to_process = self.state.jobs.drain(..).collect::<Vec<_>>(); // Take ownership
        for (idx, job) in jobs_to_process.into_iter().enumerate() {
            self.state.job_progress = (idx + 1, total_jobs);
            let result: Result<(), anyhow::Error> = match job.action {
                ActionType::Delete => {
                    match delete_files(&[job.file_info.clone()], dry_run_mode) {
                        Ok((1, logs)) => {
                            // Add logs from delete_files to our log messages
                            for log in logs {
                                self.state.log_messages.push(log);
                            }
                            Ok(())
                        }
                        Ok((count, logs)) => {
                            // Add logs anyway even when count is unexpected
                            for log in logs {
                                self.state.log_messages.push(log);
                            }
                            Err(anyhow::anyhow!(
                                "Delete action affected {} files, expected 1.",
                                count
                            ))
                        }
                        Err(e) => Err(e),
                    }
                }
                ActionType::Move(ref target_dir) => {
                    match move_files(&[job.file_info.clone()], target_dir, dry_run_mode) {
                        Ok((1, logs)) => {
                            // Add logs from move_files to our log messages
                            for log in logs {
                                self.state.log_messages.push(log);
                            }
                            Ok(())
                        }
                        Ok((count, logs)) => {
                            // Add logs anyway even when count is unexpected
                            for log in logs {
                                self.state.log_messages.push(log);
                            }
                            Err(anyhow::anyhow!(
                                "Move action affected {} files, expected 1.",
                                count
                            ))
                        }
                        Err(e) => Err(e),
                    }
                }
                ActionType::Copy(ref target_dir) => {
                    log::debug!(
                        "Attempting to copy {:?} to {:?}",
                        job.file_info.path,
                        target_dir
                    );

                    if dry_run_mode {
                        self.state.log_messages.push(format!(
                            "[DRY RUN] Would copy {} to {}",
                            job.file_info.path.display(),
                            target_dir.display()
                        ));

                        // Add more detailed logs similar to delete_files and move_files
                        if !target_dir.exists() {
                            self.state.log_messages.push(format!(
                                "[DRY RUN] Would create target directory: {}",
                                target_dir.display()
                            ));
                        }

                        // Check for potential destination conflicts (even in dry run mode)
                        let file_name = job.file_info.path.file_name().unwrap_or_default();
                        let dest_path = target_dir.join(file_name);
                        if dest_path.exists() {
                            self.state.log_messages.push(format!(
                                "[DRY RUN] Note: Destination {} exists. Would be renamed with _copy suffix",
                                dest_path.display()));
                        }

                        self.state
                            .log_messages
                            .push(format!("[DRY RUN] File size: {} bytes", job.file_info.size));

                        Ok(())
                    } else {
                        if !target_dir.exists() {
                            if let Err(e) = std::fs::create_dir_all(&target_dir) {
                                let error_msg = format!(
                                    "Failed to create target directory {}: {}",
                                    target_dir.display(),
                                    e
                                );
                                self.state.log_messages.push(error_msg);
                                log::error!(
                                    "Failed to create target directory {:?} for copy: {}",
                                    target_dir,
                                    e
                                );
                                return Err(e.into());
                            }
                            self.state
                                .log_messages
                                .push(format!("Created directory: {}", target_dir.display()));
                        }
                        let file_name = job.file_info.path.file_name().unwrap_or_default();
                        let mut dest_path = target_dir.join(file_name);
                        let mut counter = 1;
                        while dest_path.exists() {
                            let stem = dest_path.file_stem().unwrap_or_default().to_string_lossy();
                            let ext = dest_path.extension().unwrap_or_default().to_string_lossy();
                            let new_name = format!(
                                "{}_copy({}){}{}",
                                stem.trim_end_matches(&format!("_copy({})", counter - 1))
                                    .trim_end_matches("_copy"),
                                counter,
                                if ext.is_empty() { "" } else { "." },
                                ext
                            );
                            dest_path = target_dir.join(new_name);
                            counter += 1;
                        }
                        std::fs::copy(&job.file_info.path, &dest_path)
                            .map(|size| {
                                self.state.log_messages.push(format!(
                                    "Copied: {} -> {} ({} bytes)",
                                    job.file_info.path.display(),
                                    dest_path.display(),
                                    size
                                ));
                                ()
                            })
                            .map_err(|e| {
                                let error_msg = format!(
                                    "Failed to copy {}: {}",
                                    job.file_info.path.display(),
                                    e
                                );
                                self.state.log_messages.push(error_msg);
                                log::error!(
                                    "Failed to copy {:?} to {:?}: {}",
                                    job.file_info.path,
                                    dest_path,
                                    e
                                );
                                anyhow::Error::from(e)
                            })
                    }
                }
                ActionType::Keep | ActionType::Ignore => Ok(()),
            };
            if result.is_ok() {
                success_count += 1;
                if dry_run_mode {
                    self.state.log_messages.push(format!(
                        "[DRY RUN] Success: Would perform {:?} for {}",
                        job.action,
                        job.file_info.path.display()
                    ));
                } else {
                    self.state.log_messages.push(format!(
                        "Success: {:?} for {}",
                        job.action,
                        job.file_info.path.display()
                    ));
                }
            } else {
                fail_count += 1;
                self.state.log_messages.push(format!(
                    "Failed: {:?} for {}: {}",
                    job.action,
                    job.file_info.path.display(),
                    result.err().unwrap()
                ));
            }
        }
        self.state.is_processing_jobs = false;

        if dry_run_mode {
            self.state.job_processing_message = format!(
                "[DRY RUN] Simulated jobs. Success: {}, Fail: {}",
                success_count, fail_count
            );
        } else {
            self.state.job_processing_message = format!(
                "Jobs processed. Success: {}, Fail: {}",
                success_count, fail_count
            );
        }

        self.state.status_message = Some(self.state.job_processing_message.clone());
        self.state.job_progress = (0, 0);
        self.state.selected_job_index = 0;
        Ok(())
    }

    fn select_next_job(&mut self) {
        if !self.state.jobs.is_empty() {
            self.state.selected_job_index =
                (self.state.selected_job_index + 1) % self.state.jobs.len();
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
            log::info!(
                "Removed job: {:?} for file {:?}",
                removed_job.action,
                removed_job.file_info.path
            );
            if self.state.selected_job_index >= self.state.jobs.len() && !self.state.jobs.is_empty()
            {
                self.state.selected_job_index = self.state.jobs.len() - 1;
            }
            if self.state.jobs.is_empty() {
                self.state.selected_job_index = 0;
            }
            self.state.status_message = Some("Job removed.".to_string());
        } else {
            self.state.status_message =
                Some("No job selected to remove or jobs list empty.".to_string());
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
        self.state.display_list =
            App::build_display_list_from_grouped_data(&self.state.grouped_data);
        self.validate_selection_indices(); // Ensure selection is still valid
    }

    fn apply_sort_settings(&mut self) {
        log::info!(
            "Applying sort settings: {:?} {:?}",
            self.state.current_sort_criterion,
            self.state.current_sort_order
        );
        for group in &mut self.state.grouped_data {
            for set in &mut group.sets {
                // Use the utility from file_utils, assuming it's public or in the same module
                // If not, we might need to replicate or expose it.
                // For now, assuming file_utils::sort_file_infos is accessible.
                // It needs to be `pub(crate)` or public in `file_utils`.
                file_utils::sort_file_infos(
                    &mut set.files,
                    self.state.current_sort_criterion,
                    self.state.current_sort_order,
                );
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

    // Create a modified cli config with progress always enabled for TUI mode
    let mut tui_cli = cli.clone();
    tui_cli.progress = true;
    tui_cli.progress_tui = true;

    let mut app = App::new(&tui_cli);
    app.validate_selection_indices(); // Initial validation for sync loaded data if any

    // Always enable progress for TUI mode regardless of cli.progress setting
    let res = run_main_loop(&mut terminal, &mut app, true);

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
            if let Some(bt_ref) = an_option_of_backtrace {
                // bt_ref should be &std::backtrace::Backtrace
                backtrace_output = format!("Stack backtrace:\n{}", bt_ref); // &std::backtrace::Backtrace implements Display
            }
            println!("Error in TUI: {}\n{}", err, backtrace_output);
        } else {
            println!("Error in TUI: {}. Run with -vv for more details.", err);
        }
    }

    Ok(())
}

fn run_main_loop(
    terminal: &mut Terminal<TerminalBackend>,
    app: &mut App,
    show_tui_progress: bool,
) -> Result<()> {
    let tick_rate = Duration::from_millis(100); // Faster tick rate for responsiveness with async msgs
    let mut last_tick = Instant::now();

    // Handle messages from scan thread immediately for the first frame
    if show_tui_progress {
        app.handle_scan_messages();
    }

    loop {
        // Handle messages from scan thread first
        if show_tui_progress {
            // Only check messages if async scan was started
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

// Helper function to parse progress information from loading messages
fn parse_progress_from_message(message: &str) -> (String, String, Option<f64>) {
    // Extract stage from messages like "📁 [1/3] File Discovery: Found 196200 files..."
    let stage = if message.contains("[0/3]") {
        "0/3 Pre-scan".to_string()
    } else if message.contains("[1/3]") {
        "1/3 Discovery".to_string()
    } else if message.contains("[2/3]") {
        "2/3 Size Analysis".to_string()
    } else if message.contains("[3/3]") {
        "3/3 Hashing".to_string()
    } else {
        "Loading".to_string()
    };

    // Extract file counts and percentages
    let mut progress_text = message.to_string();

    // Try to extract file counts for a better display format
    if let Some(count_start) = message.find("Found ") {
        if let Some(count_end) = message[count_start..].find(" files") {
            let file_count_str = &message[count_start + 6..count_start + count_end];
            progress_text = format!("Found {} files", file_count_str);
        }
    }

    // Extract scanning path for better display
    if message.contains("Scanning:") {
        progress_text = message.to_string();
    }

    // Try to extract percentage values from messages containing them
    let percentage = if let Some(pct_start) = message.find("(") {
        if let Some(pct_end) = message[pct_start..].find("%)") {
            let pct_str = &message[pct_start + 1..pct_start + pct_end];
            pct_str.parse::<f64>().ok()
        } else {
            None
        }
    } else {
        None
    };

    (stage, progress_text, percentage)
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
            Constraint::Length(3), // Title
            Constraint::Length(3), // Status
            Constraint::Min(0),    // Main content
            Constraint::Length(5), // Log area (fixed height for now)
            Constraint::Length(1), // Progress bar (if any)
            Constraint::Length(1), // Help bar (always visible)
        ])
        .split(frame.size());

    if app.state.is_loading && app.scan_rx.is_some() {
        // Show loading screen with two progress bars - one for total progress, one for stage progress
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(20), // Upper space
                Constraint::Length(3),      // Title and global progress
                Constraint::Length(1),      // Spacing
                Constraint::Length(3),      // Stage-specific progress
                Constraint::Percentage(20), // Lower space
            ])
            .split(frame.size());

        // Extract progress information from loading message
        let (stage_str, progress_text, percentage) =
            parse_progress_from_message(&app.state.loading_message);

        // Calculate the total progress based on the stage
        let (current_stage, total_stages) = parse_stage_numbers(&stage_str);
        let total_progress = if let (Some(current), Some(total), Some(pct)) =
            (current_stage, total_stages, percentage)
        {
            // Overall progress = (completed stages + current stage progress)
            ((current - 1) as f64 / total as f64) + (pct / 100.0 / total as f64)
        } else {
            // Indeterminate if we can't extract actual values
            let now = std::time::Instant::now();
            let secs = now.elapsed().as_secs_f64();
            (secs % 2.0) / 2.0 // Pulse every 2 seconds
        };

        // Top bar: Total progress
        let total_progress_text =
            if let (Some(current), Some(total)) = (current_stage, total_stages) {
                format!(
                    "Total Progress: Stage {} of {} - {:.1}% Complete",
                    current,
                    total,
                    total_progress * 100.0
                )
            } else {
                "Processing...".to_string()
            };

        let total_progress_gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Overall Progress"),
            )
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
            .label(total_progress_text)
            .ratio(total_progress);

        frame.render_widget(total_progress_gauge, chunks[1]);

        // Stage-specific progress (bottom bar)
        // Use extracted percentage if available, otherwise animate
        let stage_progress_value = if let Some(pct) = percentage {
            pct / 100.0
        } else {
            // Animate when no percentage available
            let now = std::time::Instant::now();
            let secs = now.elapsed().as_secs_f64();
            (secs % 3.0) / 3.0 // Cycles every 3 seconds (0.0 to 1.0)
        };

        let stage_gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(stage_str))
            .gauge_style(Style::default().fg(Color::White).bg(Color::Black))
            .label(progress_text)
            .ratio(stage_progress_value);

        frame.render_widget(stage_gauge, chunks[3]);
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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Settings (Ctrl+S to enter/Esc to exit)"),
            );
        frame.render_widget(title, chunks[0]);

        let mut strategy_style = Style::default();
        let mut algo_style = Style::default();
        let mut parallel_style = Style::default();
        let mut sort_criterion_style = Style::default();
        let mut sort_order_style = Style::default();
        let mut media_mode_style = Style::default();
        let mut media_resolution_style = Style::default();
        let mut media_format_style = Style::default();
        let mut media_similarity_style = Style::default();

        match app.state.selected_setting_category_index {
            0 => {
                strategy_style = strategy_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            1 => algo_style = algo_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            2 => {
                parallel_style = parallel_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            3 => {
                sort_criterion_style = sort_criterion_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            4 => {
                sort_order_style = sort_order_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            5 => {
                media_mode_style = media_mode_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            6 => {
                media_resolution_style = media_resolution_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            7 => {
                media_format_style = media_format_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            8 => {
                media_similarity_style = media_similarity_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
            _ => {}
        }

        let settings_text = vec![
            Line::from(Span::styled(format!("1. File Selection Strategy: {:?}", app.state.default_selection_strategy), strategy_style)),
            Line::from(Span::styled(format!("   (n:newest, o:oldest, s:shortest, l:longest)"), strategy_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("2. Hashing Algorithm: {}", app.state.current_algorithm), algo_style)),
            Line::from(Span::styled(format!("   (m:md5, a:sha256, b:blake3, x:xxhash, g:gxhash, f:fnv1a, c:crc32)"), algo_style)),
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

            // Media deduplication options
            Line::from(Span::styled("--- Media Deduplication ---", Style::default().add_modifier(Modifier::BOLD))),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("6. Media Mode: {}",
                if app.state.media_mode {
                    if crate::media_dedup::is_ffmpeg_available() {
                        "Enabled"
                    } else {
                        "Enabled (ffmpeg not found, limited functionality)"
                    }
                } else {
                    "Disabled"
                }
            ), media_mode_style)),
            Line::from(Span::styled(format!("   (e:toggle, requires rescan)"), media_mode_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("7. Media Resolution Preference: {}", app.state.media_resolution), media_resolution_style)),
            Line::from(Span::styled(format!("   (h:highest, l:lowest, c:custom, requires rescan)"), media_resolution_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("8. Media Format Preference: {}",
                app.state.media_formats.iter().take(3).cloned().collect::<Vec<_>>().join(" > ")), media_format_style)),
            Line::from(Span::styled(format!("   (r:raw first, p:png first, j:jpg first, requires rescan)"), media_format_style)),
            Line::from(Span::raw("")),
            Line::from(Span::styled(format!("9. Media Similarity Threshold: {}%", app.state.media_similarity), media_similarity_style)),
            Line::from(Span::styled(format!("   (1:95% strict, 2:90% default, 3:85% relaxed, 4:75% very relaxed, requires rescan)"), media_similarity_style)),
            Line::from(Span::raw("")),
            Line::from(Span::raw(if app.state.rescan_needed && app.state.sort_settings_changed {
                "[!] Algorithm/Parallelism/Media and Sort settings changed. Ctrl+R to rescan, Sort applied on Esc."
            } else if app.state.rescan_needed {
                "[!] Algorithm/Parallelism/Media settings changed. Press Ctrl+R to rescan."
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
                Constraint::Min(0),    // Content
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
            Line::from("  Ctrl+D     : Toggle Dry Run mode (simulates actions without making changes)"),
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
            Line::from("  Algorithm  : m (md5), a (sha256), b (blake3), x (xxhash), g (gxhash), f (fnv1a), c (crc32) - requires rescan"),
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

        let footer = Paragraph::new("Press 'Esc' to close Help.").alignment(Alignment::Center);
        frame.render_widget(footer, help_chunks[2]);
    } else {
        // Main UI (3 panels + status bar)
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35), // Sets/Folders panel
                Constraint::Percentage(35), // Files panel
                Constraint::Percentage(30), // Jobs panel
            ])
            .split(chunks[2]);

        // Helper to create a block with a title and border, highlighting if active
        let create_block = |title_string: String, is_active: bool| {
            let base_style = if is_active {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title_string, base_style))
                .border_style(base_style)
        };

        // Left Panel: Duplicate Sets (actually folders and sets)
        let sets_panel_title_string = format!(
            "Parent Folders / Duplicate Sets ({}/{}) (Tab to navigate)",
            app.state
                .selected_display_list_index
                .saturating_add(1)
                .min(app.state.display_list.len()),
            app.state.display_list.len()
        );
        let sets_block = create_block(
            sets_panel_title_string,
            app.state.active_panel == ActivePanel::Sets
                && app.state.input_mode == InputMode::Normal,
        );

        let list_items: Vec<ListItem> = app
            .state
            .display_list
            .iter()
            .map(|item| match item {
                DisplayListItem::Folder {
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
                DisplayListItem::SetEntry {
                    set_hash_preview,
                    set_total_size,
                    file_count_in_set,
                    indent,
                    ..
                } => {
                    let indent_str = if *indent { "  " } else { "" };
                    ListItem::new(Line::from(Span::styled(
                        format!(
                            "{}Hash: {}... ({} files, {})",
                            indent_str,
                            set_hash_preview,
                            file_count_in_set,
                            format_file_size(*set_total_size, app.cli_config.raw_sizes)
                        ),
                        Style::default(),
                    )))
                }
            })
            .collect();

        let sets_list = List::new(list_items)
            .block(sets_block)
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
        frame.render_stateful_widget(sets_list, main_chunks[0], &mut sets_list_state);

        // Middle Panel: Files in Selected Set
        let (files_panel_title_string, file_items) = if let Some(selected_set) =
            app.current_selected_set_from_display_list()
        {
            let title = format!(
                "Files ({}/{}) (s:keep d:del c:copy i:ign h:back)",
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
                    } else {
                        if let Ok((default_kept, _)) = file_utils::determine_action_targets(
                            selected_set,
                            app.state.default_selection_strategy,
                        ) {
                            if default_kept.path == file_info.path {
                                style = style.fg(Color::Green);
                                prefix = "[k]";
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
                vec![ListItem::new("No set selected or set is empty")],
            )
        };
        let files_block = create_block(
            files_panel_title_string,
            app.state.active_panel == ActivePanel::Files
                && app.state.input_mode == InputMode::Normal,
        );
        let files_list = List::new(file_items)
            .block(files_block)
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray),
            )
            .highlight_symbol("> ");

        let mut files_list_state = ListState::default();
        if app
            .current_selected_set_from_display_list()
            .map_or(false, |s| !s.files.is_empty())
        {
            files_list_state.select(Some(app.state.selected_file_index_in_set));
        }
        frame.render_stateful_widget(files_list, main_chunks[1], &mut files_list_state);

        // Right Panel: Jobs
        let jobs_panel_title_string =
            format!("Jobs ({}) (Ctrl+E: Exec, x:del)", app.state.jobs.len());
        let jobs_block = create_block(
            jobs_panel_title_string,
            app.state.active_panel == ActivePanel::Jobs
                && app.state.input_mode == InputMode::Normal,
        );
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
            .block(jobs_block)
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
                    "q/Ctrl+C:quit | Tab:cycle | Arrows/jk:nav | a:toggle s:keep d:del c:copy i:ign | Ctrl+E:exec | Ctrl+R:rescan | Ctrl+S:settings | x:del job"
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
            InputMode::Settings => {
                // The Settings mode has its own full-screen UI, so no specific status bar here.
            }
            InputMode::Help => {
                // The Help mode has its own full-screen UI, so no specific status bar here.
            }
        }

        // Draw progress bar (if any) just above the help bar
        use ratatui::widgets::Gauge;
        if app.state.is_processing_jobs {
            let (done, total) = app.state.job_progress;
            let percent = if total > 0 {
                done as f64 / total as f64
            } else {
                0.0
            };

            // Create a progress display area for job processing
            let progress_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Top bar
                    Constraint::Length(1), // Bottom bar
                ])
                .split(chunks[4]);

            // Top gauge shows overall progress
            let top_gauge = Gauge::default()
                .block(Block::default().borders(Borders::NONE).title(""))
                .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .label(format!(
                    "Overall: {}/{} jobs ({:.1}%)",
                    done,
                    total,
                    percent * 100.0
                ))
                .ratio(percent);

            // Bottom gauge shows per-job details
            let bottom_gauge = Gauge::default()
                .block(Block::default().borders(Borders::NONE).title(""))
                .gauge_style(Style::default().fg(Color::Green).bg(Color::Black))
                .label(format!(
                    "Current job: {}/{} - {}",
                    done, total, app.state.job_processing_message
                ))
                .ratio(if done < total {
                    (done as f64 + 0.5) / total as f64
                } else {
                    1.0
                });

            frame.render_widget(top_gauge, progress_layout[0]);
            frame.render_widget(bottom_gauge, progress_layout[1]);
        } else if app.state.is_loading {
            // Extract progress information from the loading message
            let (stage_str, progress_text, percentage) =
                parse_progress_from_message(&app.state.loading_message);

            // Calculate total progress across stages
            let (current_stage, total_stages) = parse_stage_numbers(&stage_str);
            let total_progress = if let (Some(current), Some(total), Some(pct)) =
                (current_stage, total_stages, percentage)
            {
                ((current - 1) as f64 / total as f64) + (pct / 100.0 / total as f64)
            } else {
                // Animate when no percentage available
                let now = std::time::Instant::now();
                let secs = now.elapsed().as_secs_f64();
                (secs % 3.0) / 3.0 // Cycles every 3 seconds (0.0 to 1.0)
            };

            // Create a progress display area with two progress bars
            let progress_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Top bar for overall progress
                    Constraint::Length(1), // Bottom bar for stage progress
                ])
                .split(chunks[4]);

            // Top gauge shows overall progress across all stages
            let top_gauge = Gauge::default()
                .block(Block::default().borders(Borders::NONE).title(""))
                .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .label(
                    if let (Some(current), Some(total)) = (current_stage, total_stages) {
                        format!(
                            "Total Progress: Stage {} of {} ({:.1}%)",
                            current,
                            total,
                            total_progress * 100.0
                        )
                    } else {
                        "Processing...".to_string()
                    },
                )
                .ratio(total_progress);

            // Bottom gauge shows progress for current stage
            let stage_progress_value = if let Some(pct) = percentage {
                pct / 100.0
            } else if let Some(counts) = extract_scan_counts(&app.state.loading_message) {
                counts.0 as f64 / counts.1 as f64
            } else {
                // Animate when no percentage available
                let now = std::time::Instant::now();
                let secs = now.elapsed().as_secs_f64();
                (secs % 3.0) / 3.0 // Cycles every 3 seconds (0.0 to 1.0)
            };

            let bottom_gauge = Gauge::default()
                .block(Block::default().borders(Borders::NONE).title(""))
                .gauge_style(Style::default().fg(Color::White).bg(Color::Black))
                .label(progress_text)
                .ratio(stage_progress_value);

            frame.render_widget(top_gauge, progress_layout[0]);
            frame.render_widget(bottom_gauge, progress_layout[1]);
        } else if !app.state.jobs.is_empty() && app.state.input_mode == InputMode::Normal {
            let total = app.state.jobs.len();
            let completed = 0; // You can track completed jobs if you add a field
            let percent = if total > 0 {
                completed as f64 / total as f64
            } else {
                0.0
            };
            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL).title("Job Progress"))
                .gauge_style(Style::default().fg(Color::White).bg(Color::Black))
                .label(format!(
                    "Pending jobs: {} | Ctrl+E: Execute, x: Remove job",
                    total
                ))
                .ratio(percent);
            frame.render_widget(gauge, chunks[4]);
        } else {
            // Draw an empty block if no progress
            let empty = Block::default();
            frame.render_widget(empty, chunks[4]);
        }

        // Draw help bar at the very bottom
        let help =
            "h: Help | ↑/↓: Navigate | Space: Toggle | a: Toggle Keep/Delete | q/Ctrl+C: Quit";
        let help_bar = ratatui::widgets::Paragraph::new(help)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(help_bar, chunks[5]);

        // Draw log area (scrollable)
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
                    .map_or(true, |f| msg.contains(f))
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
        frame.render_widget(log_paragraph, chunks[3]);
    }
}

// Helper function to extract scan counts from loading messages
// Returns (current_count, total_count) if available
fn extract_scan_counts(message: &str) -> Option<(usize, usize)> {
    // Look for patterns like "Found 123/456 files" or "Scanned 123/456 files"
    if let Some(idx) = message.find('/') {
        let before = &message[..idx];
        let after = &message[idx + 1..];

        // Extract current count from before the slash
        let current = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .parse::<usize>()
            .ok()?;

        // Extract total count from after the slash
        let total = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<usize>()
            .ok()?;

        if total > 0 {
            return Some((current, total));
        }
    }

    None
}

// Helper function to parse stage numbers from a stage string like "1/3 Discovery"
fn parse_stage_numbers(stage_str: &str) -> (Option<usize>, Option<usize>) {
    // Look for patterns like "1/3" or "0/3"
    if let Some(idx) = stage_str.find('/') {
        if idx > 0 && idx + 1 < stage_str.len() {
            let current_str = &stage_str[..idx];
            let rest = &stage_str[idx + 1..];

            // Extract current stage number
            let current = current_str
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
                .parse::<usize>()
                .ok();

            // Extract total stages number
            let total = if let Some(space_idx) = rest.find(' ') {
                let total_str = &rest[..space_idx];
                total_str.parse::<usize>().ok()
            } else {
                rest.parse::<usize>().ok()
            };

            return (current, total);
        }
    }

    // Default if we couldn't parse the stage numbers
    (None, None)
}
