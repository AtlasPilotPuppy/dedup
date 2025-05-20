use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
// For displaying actual core count in auto mode
use std::collections::HashMap; // For grouping
use std::path::{Path, PathBuf}; // Ensure Path is imported here
use std::str::FromStr;
use std::sync::mpsc as std_mpsc; // Alias to avoid conflict if crate::mpsc is used elsewhere
use std::thread as std_thread; // Alias for clarity
use tui_input::Input;
use std::collections::HashSet;
 // For input handling

use crate::file_utils::{
    self, delete_files, move_files, DuplicateSet, FileInfo, SelectionStrategy, SortCriterion,
    SortOrder,
};
use crate::options::Options; // Using Options instead of Cli

// Add the copy_missing module
pub mod copy_missing;

// Add the file_browser module
pub mod file_browser;

// Add the explorer_browser module
pub mod explorer_browser;

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
    pub selected_destination_index: usize, // New field for destination browser
    pub destination_path: Option<PathBuf>, // Current path for destination browsing
    pub jobs: Vec<Job>,
    pub active_panel: ActivePanel,
    pub default_selection_strategy: SelectionStrategy, // Store parsed strategy
    pub status_message: Option<String>,                // For feedback
    pub input_mode: InputMode,
    pub current_input: Input,                 // Using tui-input crate
    pub file_for_copy_move: Option<FileInfo>, // Store file when prompting for dest

    // File browser - using new module
    pub file_browser: Option<crate::tui_app::file_browser::FileBrowser>,
    
    // Enhanced explorer - using ratatui-explorer
    pub enhanced_explorer: Option<crate::tui_app::explorer_browser::EnhancedExplorer>,

    // Update mode - only copy newer files
    pub update_mode: bool,

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
    pub is_copy_missing_mode: bool, // Set to true for copy missing mode
    pub selected_left_panel: HashSet<PathBuf>, // Track selected files/sets in the left panel
    pub last_job_completion_check: Option<bool> // Track if jobs were processing in the last check
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
    options_config: Options,                        // Store the initial options config
}

impl App {
    pub fn new(options: &Options) -> Self {
        log::info!(
            "Creating TUI app with progress_tui={}",
            options.progress_tui
        );

        let strategy =
            SelectionStrategy::from_str(&options.mode).unwrap_or(SelectionStrategy::NewestModified);
        let initial_status = "Preparing to scan for duplicates...";

        let app_state = AppState {
            grouped_data: Vec::new(),
            display_list: Vec::new(),
            selected_display_list_index: 0,
            selected_file_index_in_set: 0,
            selected_job_index: 0,
            selected_destination_index: 0, // New field for destination browser
            destination_path: None,        // Current path for destination browsing
            jobs: Vec::new(),
            active_panel: ActivePanel::Sets,
            default_selection_strategy: strategy,
            status_message: None,
            input_mode: InputMode::Normal,
            current_input: Input::default(),
            file_for_copy_move: None,
            is_loading: true, // Always start in loading state, scan will update
            loading_message: initial_status.to_string(),
            current_algorithm: options.algorithm.clone(),
            current_parallel: options.parallel,
            rescan_needed: false,
            selected_setting_category_index: 0,
            current_sort_criterion: options.sort_by, // Initialize from Options
            current_sort_order: options.sort_order,  // Initialize from Options
            sort_settings_changed: false,
            media_mode: options.media_mode,
            media_resolution: options.media_resolution.clone(),
            media_formats: options.media_formats.clone(),
            media_similarity: options.media_similarity,
            log_messages: Vec::new(),
            log_scroll: 0,
            log_focus: false,
            log_filter: None,
            is_processing_jobs: false,
            job_processing_message: String::new(),
            job_progress: (0, 0),
            dry_run: options.dry_run,    // Initialize from Options args
            is_copy_missing_mode: false, // Set to false for regular mode
            file_browser: None,
            update_mode: false,
            selected_left_panel: HashSet::new(),
            enhanced_explorer: None,
            last_job_completion_check: None,
        };

        // Always perform async scan for TUI
        log::info!(
            "Initializing TUI with directory: {:?}",
            options.directories[0]
        );

        let (tx, rx) = std_mpsc::channel::<ScanMessage>();

        // Send an immediate status update to show we're properly initialized
        if let Err(e) = tx.send(ScanMessage::StatusUpdate(
            1,
            format!("Starting scan of {}...", options.directories[0].display()),
        )) {
            log::error!("Failed to send initial status update: {}", e);
        }

        let mut current_options_for_scan = options.clone();
        current_options_for_scan.algorithm = app_state.current_algorithm.clone();
        current_options_for_scan.parallel = app_state.current_parallel;
        current_options_for_scan.sort_by = app_state.current_sort_criterion;
        current_options_for_scan.sort_order = app_state.current_sort_order;

        // Ensure progress and progress_tui are always set to true for TUI mode
        current_options_for_scan.progress = true;
        current_options_for_scan.progress_tui = true;

        log::debug!(
            "Using scan options: progress={}, progress_tui={}",
            current_options_for_scan.progress,
            current_options_for_scan.progress_tui
        );

        log::info!(
            "Starting scan thread with algorithm={}, parallel={:?}",
            current_options_for_scan.algorithm,
            current_options_for_scan.parallel
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
                    log::error!("[ScanThread] Failed to send initialization message: {}", e);
                });

            match file_utils::find_duplicate_files_with_progress(
                &current_options_for_scan,
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
        let id = scan_thread.thread().id();
        let scan_join_handle = {
            log::info!("Scan thread started with ID: {:?}", id);
            Some(scan_thread)
        };

        Self {
            state: app_state,
            should_quit: false,
            scan_thread_join_handle: scan_join_handle,
            scan_rx: Some(rx),
            scan_tx: Some(tx),
            options_config: options.clone(),
        }
    }

    pub fn new_copy_missing_mode(options: &Options) -> Self {
        // Start with regular initialization
        let mut app = Self::new(options);

        // Override for copy missing mode
        app.state.is_copy_missing_mode = true;
        app.state.status_message =
            Some("Copy Missing Mode - Preparing to scan for files to copy...".to_string());
            
        // Initialize destination to the target directory if specified
        if let Some(target) = &options.target {
            app.state.destination_path = Some(target.clone());
        } else if !options.directories.is_empty() && options.directories.len() > 1 {
            // Use the second directory as destination by default in copy missing mode
            app.state.destination_path = Some(options.directories[1].clone());
        }

        app
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

        let mut current_options_for_scan = self.options_config.clone(); // Use stored options_config
        current_options_for_scan.algorithm = self.state.current_algorithm.clone();
        current_options_for_scan.parallel = self.state.current_parallel;
        current_options_for_scan.sort_by = self.state.current_sort_criterion;
        current_options_for_scan.sort_order = self.state.current_sort_order;
        // Always enable progress for TUI mode
        current_options_for_scan.progress = true;
        current_options_for_scan.progress_tui = true;

        // Apply media deduplication options
        current_options_for_scan.media_mode = self.state.media_mode;
        current_options_for_scan.media_resolution = self.state.media_resolution.clone();
        current_options_for_scan.media_formats = self.state.media_formats.clone();
        current_options_for_scan.media_similarity = self.state.media_similarity;

        // If media mode is enabled, set up the media_dedup_options
        if current_options_for_scan.media_mode {
            // Clear any existing options first
            current_options_for_scan.media_dedup_options =
                crate::media_dedup::MediaDedupOptions::default();

            // Apply settings to media_dedup_options
            crate::media_dedup::add_media_options_to_cli(
                &mut current_options_for_scan.media_dedup_options,
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
                &current_options_for_scan,
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

        let id = scan_thread.thread().id();
        let scan_join_handle = {
            log::info!("Scan thread started with ID: {:?}", id);
            Some(scan_thread)
        };

        self.scan_thread_join_handle = scan_join_handle;
        self.scan_tx = Some(tx);
    }

    // Method to handle messages from the scan thread
    pub fn handle_scan_messages(&mut self) {
        if let Some(ref rx) = self.scan_rx {
            log::trace!("Checking for scan messages");

            match rx.try_recv() {
                Ok(message) => {
                    log::debug!(
                        "Received scan message: {:?}",
                        match &message {
                            ScanMessage::StatusUpdate(stage, msg) =>
                                format!("Status({}): {}", stage, msg),
                            ScanMessage::Completed(_) => "Completed".to_string(),
                            ScanMessage::Error(e) => format!("Error: {}", e),
                        }
                    );

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

                                    // Also add to log messages for persistence
                                    self.state.log_messages.push(format!(
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
                                    self.state.log_messages.push(format!("Scan failed: {}", e));
                                }
                            }
                        }
                        ScanMessage::Error(err) => {
                            log::error!("Scan error: {}", err);

                            self.state.is_loading = false;
                            self.state.status_message = Some(format!("Scan error: {}", err));
                            self.state.log_messages.push(format!("Scan error: {}", err));
                        }
                    }
                }
                Err(std_mpsc::TryRecvError::Empty) => {
                    // No messages available, perfectly normal.
                    log::trace!("No scan messages in queue");
                }
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    // Channel disconnected. This could happen if the scan thread finishes.
                    log::warn!("Scan thread channel disconnected.");

                    if self.state.is_loading {
                        // If still in loading state, this is an error.
                        self.state.is_loading = false;
                        self.state.status_message =
                            Some("Scan thread disconnected unexpectedly.".to_string());
                        self.state
                            .log_messages
                            .push("Scan thread disconnected unexpectedly.".to_string());
                    }
                }
            }
        } else {
            log::warn!("No scan_rx receiver available");
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
                if self.state.is_copy_missing_mode {
                    // In copy missing mode, 'C' means copy the selected file to destination
                    self.initiate_copy_missing_action();
                } else {
                    // Regular mode - initiate copy action (existing implementation)
                    self.initiate_copy_action();
                }
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
            // Handle simple cases directly, as tui-input's API for cursor manipulation is limited
            KeyCode::Char(c) => {
                self.state.current_input = tui_input::Input::from(self.state.current_input.value().to_string() + &c.to_string());
            }
            KeyCode::Backspace => {
                let value = self.state.current_input.value().to_string();
                if !value.is_empty() {
                    self.state.current_input = tui_input::Input::from(&value[..value.len()-1]);
                }
            }
            _ => {} // Ignore other keys for simplicity
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
        // For copy_missing mode, use the new start_job_execution method
        if self.state.is_copy_missing_mode {
            // Use a copy instead of a reference to avoid the borrow issue
            let options_copy = self.options_config.clone();
            self.start_job_execution(&options_copy);
            return Ok(());
        }

        // Original implementation for regular TUI mode
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
                            if let Err(e) = std::fs::create_dir_all(target_dir) {
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
        if key_event.code == KeyCode::Esc {
            self.state.input_mode = InputMode::Normal;
            self.state.status_message = Some("Exited help screen.".to_string());
        }
        // Other keys do nothing in help mode
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

    // New method for processing selected files in copy missing mode
    fn initiate_copy_missing_action(&mut self) {
        if self.state.active_panel == ActivePanel::Sets {
            // Get the selected item from display list
            if let Some(item) = self
                .state
                .display_list
                .get(self.state.selected_display_list_index)
            {
                match item {
                    DisplayListItem::SetEntry { 
                        original_group_index, 
                        original_set_index_in_group,
                        .. 
                    } => {
                        // Get the actual file from the grouped data
                        if let Some(group) = self.state.grouped_data.get(*original_group_index) {
                            if let Some(set) = group.sets.get(*original_set_index_in_group) {
                                if let Some(file) = set.files.first() {
                                    // The first file is the one missing
                                    if let Some(dest_path) = &self.state.destination_path {
                                        // Create a job to copy the file to the destination
                                        self.state.jobs.push(Job {
                                            action: ActionType::Copy(dest_path.clone()),
                                            file_info: file.clone(),
                                        });
                                        
                                        self.state.status_message = Some(format!(
                                            "Added job: Copy {} to {}",
                                            file.path.display(),
                                            dest_path.display()
                                        ));
                                        
                                        // Switch to jobs panel to show the new job
                                        self.state.active_panel = ActivePanel::Jobs;
                                        self.state.selected_job_index = self.state.jobs.len() - 1;
                                    } else {
                                        // No destination selected yet
                                        self.state.status_message = Some(
                                            "Please select a destination directory first"
                                                .to_string(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    DisplayListItem::Folder { .. } => {
                        // Folders cannot be copied directly, must select a file
                        self.state.status_message =
                            Some("Please select a specific file to copy".to_string());
                }
            }
        }
    }
}

    // Add this new method
    pub fn start_job_execution(&mut self, _options: &Options) {
        if self.state.jobs.is_empty() {
            self.state.status_message = Some("No jobs to process.".to_string());
            self.state
                .log_messages
                .push("No jobs to process.".to_string());
        return;
    }

        // Set the dry_run flag based on app state
        let dry_run_mode = self.state.dry_run;
        if dry_run_mode {
            self.state
                .log_messages
                .push("DRY RUN MODE: Simulating actions without making changes".to_string());
        }

        if self.state.update_mode {
            self.state
                .log_messages
                .push("UPDATE MODE: Only copying newer files".to_string());
        }

        self.state.is_processing_jobs = true;

        let status_prefix = match (dry_run_mode, self.state.update_mode) {
            (true, true) => "[DRY RUN][UPDATE MODE]",
            (true, false) => "[DRY RUN]",
            (false, true) => "[UPDATE MODE]",
            (false, false) => "",
        };

        self.state.job_processing_message = format!("{} Processing jobs...", status_prefix);
        self.state.status_message = Some(self.state.job_processing_message.clone());

        let total_jobs = self.state.jobs.len();
        self.state.job_progress = (0, total_jobs);

        // Group jobs by action type
        let mut copy_jobs = Vec::new();
        let mut move_jobs = Vec::new();
        let mut delete_jobs = Vec::new();

        for job in &self.state.jobs {
            match &job.action {
                ActionType::Copy(dest) => copy_jobs.push((dest, &job.file_info)),
                ActionType::Move(dest) => move_jobs.push((dest, &job.file_info)),
                ActionType::Delete => delete_jobs.push(&job.file_info),
                _ => {} // Ignore other job types
            }
        }

        let mut success_count = 0;
        let mut updated_count = 0;
        let mut skipped_count = 0;
        let mut fail_count = 0;

        // Execute COPY jobs
        if !copy_jobs.is_empty() {
            // Group copy jobs by destination for efficiency
            let mut by_dest: std::collections::HashMap<&std::path::Path, Vec<&FileInfo>> =
                std::collections::HashMap::new();
            for (dest, file) in copy_jobs {
                by_dest.entry(dest.as_path()).or_default().push(file);
            }

            for (dest, files) in by_dest {
                // Update status message
                self.state.status_message =
                    Some(format!("{} Copying to {}", status_prefix, dest.display()));

                // Convert to owned FileInfo objects
                let owned_files: Vec<FileInfo> = files.iter().map(|&f| f.clone()).collect();

                if self.state.update_mode {
                    // Use update_mode for copying (only newer files)
                    match crate::update_mode::update_files(&owned_files, dest, dry_run_mode, None) {
                        Ok(result) => {
                            // Add all log messages
                            self.state.log_messages.extend(result.log_messages);

                            // Add any errors
                            for err in result.errors {
                                self.state.log_messages.push(format!("ERROR: {}", err));
                                fail_count += 1;
                            }

                            // Count successes
                            success_count += result.copied_files;
                            updated_count += result.updated_files;
                            skipped_count += result.skipped_files;
                        }
                        Err(e) => {
                            fail_count += files.len();
                            self.state
                                .log_messages
                                .push(format!("ERROR during update: {}", e));
                        }
                    }
        } else {
                    // Use regular copy for all files
                    match crate::file_utils::copy_missing_files(&owned_files, dest, dry_run_mode) {
                        Ok((count, logs)) => {
                            success_count += count;
                            self.state.log_messages.extend(logs);
                        }
                        Err(e) => {
                            fail_count += files.len();
                            self.state
                                .log_messages
                                .push(format!("ERROR during copy: {}", e));
                        }
                    }
                }
            }
        }

        // Execute MOVE jobs
        if !move_jobs.is_empty() {
            // Group by destination
            let mut by_dest: std::collections::HashMap<&std::path::Path, Vec<&FileInfo>> =
                std::collections::HashMap::new();
            for (dest, file) in move_jobs {
                by_dest.entry(dest.as_path()).or_default().push(file);
            }

            for (dest, files) in by_dest {
                self.state.status_message =
                    Some(format!("{} Moving to {}", status_prefix, dest.display()));

                // Convert to owned FileInfo objects
                let owned_files: Vec<FileInfo> = files.iter().map(|&f| f.clone()).collect();

                match crate::file_utils::move_files(&owned_files, dest, dry_run_mode) {
                    Ok((count, logs)) => {
                        success_count += count;
                        self.state.log_messages.extend(logs);
                    }
                    Err(e) => {
                        fail_count += files.len();
                        self.state
            .log_messages
                            .push(format!("ERROR during move: {}", e));
                    }
                }
            }
        }

        // Execute DELETE jobs
        if !delete_jobs.is_empty() {
            self.state.status_message = Some(format!("{} Deleting files", status_prefix));

            // Convert to owned FileInfo objects
            let owned_files: Vec<FileInfo> = delete_jobs.iter().map(|&f| f.clone()).collect();

            match crate::file_utils::delete_files(&owned_files, dry_run_mode) {
                Ok((count, logs)) => {
                    success_count += count;
                    self.state.log_messages.extend(logs);
                }
                Err(e) => {
                    fail_count += delete_jobs.len();
                    self.state
                        .log_messages
                        .push(format!("ERROR during delete: {}", e));
                }
            }
        }

        // Update progress
        self.state.job_progress.0 = total_jobs;
        self.state.is_processing_jobs = false;

        // Create summary message
        let mut summary = String::new();

        if dry_run_mode {
            summary.push_str("[DRY RUN] Would have ");
        }

        if self.state.update_mode {
            summary.push_str(&format!(
                "processed {} jobs: {} copied, {} updated, {} skipped, {} failed",
                total_jobs, success_count, updated_count, skipped_count, fail_count
            ));
        } else {
            summary.push_str(&format!(
                "processed {} jobs: {} succeeded, {} failed",
                total_jobs, success_count, fail_count
            ));
        }

        self.state.job_processing_message = summary.clone();
        self.state.status_message = Some(summary);
        self.state.jobs.clear();
        self.state.selected_job_index = 0;
    }
}
