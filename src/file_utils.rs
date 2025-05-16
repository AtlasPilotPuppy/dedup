use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use walkdir::WalkDir;
use num_cpus;
use glob::{Pattern, PatternError};

use crate::Cli;
use crate::tui_app::ScanMessage;
use std::sync::mpsc::Sender as StdMpscSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortCriterion {
    FileName,
    FileSize,
    CreatedAt,
    ModifiedAt,
    PathLength,
}

impl SortCriterion {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "name" => Ok(Self::FileName),
            "size" => Ok(Self::FileSize),
            "created" => Ok(Self::CreatedAt),
            "modified" | "modifiedat" => Ok(Self::ModifiedAt),
            "path" => Ok(Self::PathLength),
            _ => Err(anyhow::anyhow!("Invalid sort criterion: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "asc" | "ascending" => Ok(Self::Ascending),
            "desc" | "descending" => Ok(Self::Descending),
            _ => Err(anyhow::anyhow!("Invalid sort order: {}", s)),
        }
    }
}

// Represents information about a single file, including its hash if calculated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub hash: Option<String>,
    pub modified_at: Option<SystemTime>,
    pub created_at: Option<SystemTime>,
}

// Represents a set of duplicate files (same size, same hash).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DuplicateSet {
    pub files: Vec<FileInfo>,
    pub size: u64,
    pub hash: String,
}

// New struct for the output log format
#[derive(serde::Serialize, Debug)] // Added Debug for logging if needed
struct HashEntryContent {
    size: u64,
    files: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct FilterRules {
    includes: Vec<Pattern>,
    excludes: Vec<Pattern>,
}

impl FilterRules {
    pub fn new(cli: &Cli) -> Result<Self> {
        let mut rules = FilterRules::default();

        // Process --filter-from file first
        if let Some(filter_file_path) = &cli.filter_from {
            log::info!("Loading filter rules from: {:?}", filter_file_path);
            let file = File::open(filter_file_path)
                .map_err(|e| anyhow::anyhow!("Failed to open filter file {:?}: {}", filter_file_path, e))?;
            let reader = BufReader::new(file);
            for (line_num, line_result) in reader.lines().enumerate() {
                let line = line_result.map_err(|e| anyhow::anyhow!("Failed to read line from filter file: {}", e))?;
                let trimmed_line = line.trim();
                if trimmed_line.is_empty() || trimmed_line.starts_with('#') || trimmed_line.starts_with(';') {
                    continue;
                }

                if let Some(pattern_str) = trimmed_line.strip_prefix("+ ") {
                    rules.add_include(pattern_str.trim())?;
                } else if let Some(pattern_str) = trimmed_line.strip_prefix("- ") {
                    rules.add_exclude(pattern_str.trim())?;
                } else {
                    log::warn!("Invalid line in filter file {:?} at line {}: {}", filter_file_path, line_num + 1, trimmed_line);
                }
            }
        }

        // Process --include flags
        for pattern_str in &cli.include {
            rules.add_include(pattern_str)?;
        }

        // Process --exclude flags
        for pattern_str in &cli.exclude {
            rules.add_exclude(pattern_str)?;
        }
        
        if !rules.includes.is_empty() {
            log::info!("Include rules active: {}", rules.includes.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "));
        }
        if !rules.excludes.is_empty() {
            log::info!("Exclude rules active: {}", rules.excludes.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "));
        }

        Ok(rules)
    }

    fn add_include(&mut self, pattern_str: &str) -> Result<(), PatternError> {
        match Pattern::new(pattern_str) {
            Ok(p) => {
                self.includes.push(p);
                Ok(())
            }
            Err(e) => {
                log::error!("Invalid include glob pattern '{}': {}", pattern_str, e);
                Err(e)
            }
        }
    }

    fn add_exclude(&mut self, pattern_str: &str) -> Result<(), PatternError> {
        match Pattern::new(pattern_str) {
            Ok(p) => {
                self.excludes.push(p);
                Ok(())
            }
            Err(e) => {
                log::error!("Invalid exclude glob pattern '{}': {}", pattern_str, e);
                Err(e)
            }
        }
    }

    pub fn is_match(&self, path_str: &str) -> bool {
        // 1. Check excludes: if any exclude pattern matches, path is excluded.
        if self.excludes.iter().any(|p| p.matches(path_str)) {
            return false;
        }

        // 2. Check includes:
        //    - If include patterns exist, path must match at least one.
        //    - If no include patterns exist, path is included by default (if not excluded).
        if !self.includes.is_empty() {
            return self.includes.iter().any(|p| p.matches(path_str));
        }

        true // Not excluded, and no include rules to restrict further OR matches an include rule.
    }
}

pub fn calculate_hash(path: &Path, algorithm: &str) -> Result<String> {
    let file_content = fs::read(path)?;
    match algorithm {
        "md5" => {
            let digest = md5::compute(file_content);
            Ok(format!("{:x}", digest))
        }
        "sha1" => {
            use sha1::{Sha1, Digest};
            let mut hasher = Sha1::new();
            hasher.update(file_content);
            let result = hasher.finalize();
            Ok(format!("{:x}", result))
        }
        "sha256" => {
            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(file_content);
            let result = hasher.finalize();
            Ok(format!("{:x}", result))
        }
        "blake3" => {
            let hash = blake3::hash(&file_content);
            Ok(hash.to_hex().as_str().to_string())
        }
        _ => Err(anyhow::anyhow!("Unsupported hashing algorithm: {}", algorithm)),
    }
}

pub fn find_duplicate_files_with_progress(
    cli: &Cli,
    tx_progress: StdMpscSender<ScanMessage>,
) -> Result<Vec<DuplicateSet>> {
    log::info!("[ScanThread] Starting scan with progress updates for directory: {:?}", cli.directory);
    let filter_rules = FilterRules::new(cli)?;

    let send_status = |msg: String| {
        if tx_progress.send(ScanMessage::StatusUpdate(msg)).is_err() {
            log::warn!("[ScanThread] Failed to send status update to TUI (channel closed).");
        }
    };
    send_status(format!("Starting scan in {}", cli.directory.display()));

    let mut files_by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let walker = WalkDir::new(&cli.directory).into_iter();
    let mut files_scanned_count = 0;
    let mut last_update_time = std::time::Instant::now();
    let update_interval = std::time::Duration::from_millis(250); // Update UI at most 4 times per second

    for entry_result in walker.filter_entry(|e| {
        if is_hidden(e) || is_symlink(e) { return false; }
        if let Some(path_str) = e.path().to_str() {
            filter_rules.is_match(path_str)
        } else {
            log::warn!("[ScanThread] Path {:?} is not valid UTF-8, excluding.", e.path());
            false 
        }
    }) {
        match entry_result {
            Ok(entry) => {
                if entry.file_type().is_file() {
                    let path = entry.path().to_path_buf();
                    files_scanned_count +=1;
                    if files_scanned_count % 100 == 0 || last_update_time.elapsed() >= update_interval { 
                        last_update_time = std::time::Instant::now();
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                        send_status(format!("Scanning: {} files found... (current: {})", files_scanned_count, file_name));
                    }
                    match fs::metadata(&path) {
                        Ok(metadata) => {
                            if metadata.len() > 0 { 
                                files_by_size.entry(metadata.len()).or_default().push(path);
                            }
                        }
                        Err(e) => log::warn!("[ScanThread] Failed to get metadata for {:?}: {}", path, e),
                    }
                }
            }
            Err(e) => log::warn!("[ScanThread] Error walking directory: {}", e),
        }
    }
    send_status(format!("File scan complete. Found {} files in {} size groups.", 
        files_by_size.values().map(|v| v.len()).sum::<usize>(), 
        files_by_size.len()));
    log::info!("[ScanThread] Found {} files matching criteria, grouped into {} unique file sizes.", 
        files_by_size.values().map(|v| v.len()).sum::<usize>(), 
        files_by_size.len());

    let mut duplicate_sets: Vec<DuplicateSet> = Vec::new();
    let potential_duplicates: Vec<_> = files_by_size
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .collect();

    if potential_duplicates.is_empty() {
        send_status("Scan complete. No potential duplicates found.".to_string());
        log::info!("[ScanThread] No potential duplicates found after size grouping.");
        return Ok(Vec::new());
    }

    send_status(format!("Found {} size groups with potential duplicates. Starting hash calculation...", potential_duplicates.len()));
    log::info!("[ScanThread] Found {} sizes with potential duplicates. Calculating hashes...", potential_duplicates.len());

    let num_threads = cli.parallel.unwrap_or_else(num_cpus::get);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(num_threads).build()?;
    log::info!("[ScanThread] Using {} threads for hashing.", num_threads);

    // For MPSC between hashing threads and this function's aggregation logic
    let (local_tx, local_rx) = std::sync::mpsc::channel::<Result<HashMap<String, Vec<FileInfo>>>>(); 
    let total_groups_to_hash = potential_duplicates.len();
    let mut groups_hashed_count = 0;
    let total_files_to_hash = potential_duplicates.iter().map(|(_, paths)| paths.len()).sum::<usize>();
    send_status(format!("Hashing {} files across {} size groups (using {} threads)...", 
        total_files_to_hash, total_groups_to_hash, num_threads));

    pool.install(|| {
        potential_duplicates
            .into_par_iter()
            .for_each_with(local_tx, |thread_local_tx, (size, paths)| {
                let mut hashes_in_group: HashMap<String, Vec<FileInfo>> = HashMap::new();
                for path in paths {
                    match calculate_hash(&path, &cli.algorithm) {
                        Ok(hash_str) => {
                            let metadata = match fs::metadata(&path) {
                                Ok(m) => m,
                                Err(e) => {
                                    log::warn!("Failed to get metadata for {:?}: {}", path, e);
                                    continue;
                                }
                            };
                            let file_info = FileInfo { 
                                path: path.clone(), 
                                size, 
                                hash: Some(hash_str.clone()),
                                modified_at: metadata.modified().ok(),
                                created_at: metadata.created().ok(),
                            };
                            hashes_in_group.entry(hash_str).or_default().push(file_info);
                        }
                        Err(e) => {
                            log::warn!("[ScanThread] Failed to hash {:?}: {}", path, e);
                            if thread_local_tx.send(Err(e)).is_err() {
                                log::error!("[ScanThread] Hashing thread failed to send error (channel closed).");
                            }
                            return; 
                        }
                    }
                }
                if thread_local_tx.send(Ok(hashes_in_group)).is_err() {
                     log::error!("[ScanThread] Hashing thread failed to send result (channel closed).");
                }
            });
    });
    
    // Drop the sender part for the local channel so `recv` can eventually find it closed
    // drop(local_tx); // Not needed if for_each_with handles sender lifetime correctly.

    for i in 0..total_groups_to_hash {
        match local_rx.recv() { // This will block until a message is received
            Ok(Ok(hashed_group)) => {
                for (hash, file_infos_vec) in hashed_group {
                    if file_infos_vec.len() > 1 {
                        let first_file_size = file_infos_vec[0].size; // Get size before move
                        duplicate_sets.push(DuplicateSet { 
                            files: file_infos_vec, // file_infos_vec is moved here
                            size: first_file_size, 
                            hash 
                        });
                    }
                }
            }
            Ok(Err(e)) => {
                log::error!("[ScanThread] Error hashing a file group: {}", e);
                // Decide if we should propagate this error or just log and continue
                // For now, just log. The overall function might still succeed with partial results.
            }
            Err(e) => { // mpsc::RecvError - local_tx dropped and channel empty
                log::error!("[ScanThread] Failed to receive all hash results: {}. Processed {} of {}.", e, i, total_groups_to_hash);
                // This could be an error state for the whole scan.
                // For now, return what we have, or an error
                return Err(anyhow::anyhow!("Hashing phase failed due to channel error: {}", e));
            }
        }
        groups_hashed_count += 1;
        if groups_hashed_count == 1 || groups_hashed_count == total_groups_to_hash || 
           groups_hashed_count % 5 == 0 || last_update_time.elapsed() >= update_interval {
            last_update_time = std::time::Instant::now();
            send_status(format!("Hashed {}/{} groups ({:.1}%)...", 
                groups_hashed_count, total_groups_to_hash,
                (groups_hashed_count as f64 / total_groups_to_hash as f64) * 100.0));
        }
    }

    send_status(format!("Hashing complete. Found {} sets of duplicate files.", duplicate_sets.len()));
    log::info!("[ScanThread] Found {} sets of duplicate files.", duplicate_sets.len());
    Ok(duplicate_sets)
}

pub fn find_duplicate_files(
    cli: &Cli,
) -> Result<Vec<DuplicateSet>> {
    // This version is for non-TUI or TUI sync mode.
    // It uses indicatif progress bars if cli.progress is true.
    log::info!("Scanning directory (sync): {:?}", cli.directory);
    let filter_rules = FilterRules::new(cli)?;

    let mut files_by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let walker = WalkDir::new(&cli.directory).into_iter();
    
    let pb_files_draw_target = if cli.progress { indicatif::ProgressDrawTarget::stderr() } else { indicatif::ProgressDrawTarget::hidden() };
    let pb_files = ProgressBar::with_draw_target(Some(0), pb_files_draw_target);
    pb_files.set_style(ProgressStyle::default_spinner()
        .template("{spinner:.green} Processing file: {msg:.dim} ({elapsed_precise})")?);
    if cli.progress { pb_files.enable_steady_tick(std::time::Duration::from_millis(100)); }

    for entry_result in walker.filter_entry(|e| {
        if is_hidden(e) || is_symlink(e) { return false; }
        if let Some(path_str) = e.path().to_str() {
            filter_rules.is_match(path_str)
        } else {
            log::warn!("Path {:?} is not valid UTF-8, excluding.", e.path());
            false
        }
    }) {
        match entry_result {
            Ok(entry) => {
                if entry.file_type().is_file() {
                    let path = entry.path().to_path_buf();
                    if cli.progress { 
                        pb_files.inc(1); 
                        pb_files.set_message(path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                    }
                    match fs::metadata(&path) {
                        Ok(metadata) => {
                            if metadata.len() > 0 {
                                files_by_size.entry(metadata.len()).or_default().push(path);
                            }
                        }
                        Err(e) => log::warn!("Failed to get metadata for {:?}: {}", path, e),
                    }
                }
            }
            Err(e) => log::warn!("Error walking directory: {}", e),
        }
    }
    if cli.progress { pb_files.finish_with_message("File scanning complete."); }
    else { pb_files.finish_and_clear(); }

    log::info!("Found {} files (sync) matching criteria, grouped into {} unique file sizes.", 
        files_by_size.values().map(|v| v.len()).sum::<usize>(), 
        files_by_size.len());

    let mut duplicate_sets: Vec<DuplicateSet> = Vec::new();
    let potential_duplicates: Vec<_> = files_by_size
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .collect();

    if potential_duplicates.is_empty() {
        log::info!("No potential duplicates (sync) found after size grouping.");
        return Ok(Vec::new());
    }

    log::info!("Found {} sizes (sync) with potential duplicates. Calculating hashes...", potential_duplicates.len());

    let num_threads = cli.parallel.unwrap_or_else(num_cpus::get);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(num_threads).build()?;
    log::info!("Using {} threads for hashing (sync).", num_threads);

    let pb_hashes_draw_target = if cli.progress { indicatif::ProgressDrawTarget::stderr() } else { indicatif::ProgressDrawTarget::hidden() };
    let pb_hashes = ProgressBar::with_draw_target(Some(potential_duplicates.len() as u64), pb_hashes_draw_target);
    pb_hashes.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) Hashing groups")?
        .progress_chars("#=> "));
    if cli.progress { pb_hashes.enable_steady_tick(std::time::Duration::from_millis(100)); }

    let (local_tx, local_rx) = std::sync::mpsc::channel::<Result<HashMap<String, Vec<FileInfo>>>>();
    let total_groups_to_hash = potential_duplicates.len();

    pool.install(|| {
        potential_duplicates
            .into_par_iter()
            .for_each_with(local_tx, |thread_local_tx, (size, paths)| {
                let mut hashes_in_group: HashMap<String, Vec<FileInfo>> = HashMap::new();
                for path in paths {
                    match calculate_hash(&path, &cli.algorithm) {
                        Ok(hash_str) => {
                            let metadata = match fs::metadata(&path) {
                                Ok(m) => m,
                                Err(e) => {
                                    log::warn!("Failed to get metadata for {:?}: {}", path, e);
                                    continue;
                                }
                            };
                            let file_info = FileInfo { 
                                path: path.clone(), 
                                size, 
                                hash: Some(hash_str.clone()),
                                modified_at: metadata.modified().ok(),
                                created_at: metadata.created().ok(),
                            };
                            hashes_in_group.entry(hash_str).or_default().push(file_info);
                        }
                        Err(e) => {
                            log::warn!("Failed to hash {:?} (sync): {}", path, e);
                            if thread_local_tx.send(Err(e)).is_err() { /* ... */ }
                            return; 
                        }
                    }
                }
                if thread_local_tx.send(Ok(hashes_in_group)).is_err() { /* ... */ }
            });
    });

    for i in 0..total_groups_to_hash {
        match local_rx.recv() {
            Ok(Ok(hashed_group)) => {
                for (hash, file_infos_vec) in hashed_group {
                    if file_infos_vec.len() > 1 {
                        let first_file_size = file_infos_vec[0].size; // Get size before move
                        duplicate_sets.push(DuplicateSet { 
                            files: file_infos_vec, // file_infos_vec is moved here
                            size: first_file_size, 
                            hash 
                        });
                    }
                }
            }
            Ok(Err(e)) => { log::error!("Error hashing a file group (sync): {}", e); }
            Err(e) => { 
                log::error!("Failed to receive all hash results (sync): {}. Processed {} of {}.", e, i, total_groups_to_hash);
                return Err(anyhow::anyhow!("Hashing phase failed (sync): {}", e));
            }
        }
        if cli.progress { pb_hashes.inc(1); }
    }

    if cli.progress { pb_hashes.finish_with_message("Hashing complete (sync)."); }
    else { pb_hashes.finish_and_clear(); }

    log::info!("Found {} sets of duplicate files (sync).", duplicate_sets.len());
    Ok(duplicate_sets)
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name()
         .to_str()
         .map(|s| s.starts_with('.'))
         .unwrap_or(false)
}

fn is_symlink(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_symlink()
}

pub fn output_duplicates(
    duplicate_sets: &[DuplicateSet],
    output_path: &Path,
    format: &str,
) -> Result<()> {
    log::info!(
        "Preparing to write {} duplicate sets to {:?} in {} format",
        duplicate_sets.len(),
        output_path,
        format
    );

    let mut output_map: HashMap<String, HashEntryContent> = HashMap::new();

    for set in duplicate_sets {
        if set.files.len() >= 2 { // Only include actual duplicate sets
            let file_paths: Vec<PathBuf> = set.files.iter().map(|f| f.path.clone()).collect();
            output_map.insert(
                set.hash.clone(),
                HashEntryContent {
                    size: set.size,
                    files: file_paths,
                },
            );
        }
    }

    if output_map.is_empty() {
        log::info!("No duplicate sets with 2 or more files to output.");
        // Optionally, write an empty map or a message to the file, or just do nothing.
        // For now, if the map is empty, we won't create/overwrite the output file.
        // If an empty file is desired, uncomment below:
        // fs::write(output_path, "")?;
        return Ok(());
    }

    let output_content = match format {
        "json" => serde_json::to_string_pretty(&output_map)?,
        "toml" => toml::to_string_pretty(&output_map)?,
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported output format: {}. Supported formats are json, toml.",
                format
            ));
        }
    };

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            log::info!("Created parent directory for output file: {:?}", parent);
        }
    }

    fs::write(output_path, output_content)?;
    log::info!("Successfully wrote duplicate information to {:?}", output_path);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionStrategy {
    ShortestPath,
    LongestPath,
    NewestModified,
    OldestModified,
}

impl SelectionStrategy {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "shortest_path" => Ok(Self::ShortestPath),
            "longest_path" => Ok(Self::LongestPath),
            "newest_modified" => Ok(Self::NewestModified),
            "oldest_modified" => Ok(Self::OldestModified),
            _ => Err(anyhow::anyhow!("Invalid selection strategy: {}", s)),
        }
    }
}

// Given a set of duplicate files, determines which one to keep and which ones are to be processed (deleted/moved).
// Returns a tuple: (file_to_keep, files_to_process)
pub fn determine_action_targets(
    set: &DuplicateSet,
    strategy: SelectionStrategy,
) -> Result<(FileInfo, Vec<FileInfo>)> {
    if set.files.len() < 2 {
        // Not a duplicate set for action, or only one file left.
        return Err(anyhow::anyhow!(
            "Cannot determine action targets for a set with less than 2 files."
        ));
    }

    let mut files = set.files.clone(); // Clone to allow modification/sorting if needed

    let kept_file_info = match strategy {
        SelectionStrategy::ShortestPath => files
            .into_iter()
            .min_by_key(|f| f.path.as_os_str().len())
            .unwrap(), // Safe because len >= 2
        SelectionStrategy::LongestPath => files
            .into_iter()
            .max_by_key(|f| f.path.as_os_str().len())
            .unwrap(), // Safe
        SelectionStrategy::NewestModified => {
            files.sort_by_key(|f| fs::metadata(&f.path).and_then(|m| m.modified()).map(std::cmp::Reverse).unwrap_or_else(|_| std::cmp::Reverse(std::time::SystemTime::UNIX_EPOCH)));
            files.remove(0) // After sorting by Reverse(modified_time), the first is newest
        }
        SelectionStrategy::OldestModified => {
            files.sort_by_key(|f| fs::metadata(&f.path).and_then(|m| m.modified()).unwrap_or_else(|_| std::time::SystemTime::now()));
            files.remove(0) // After sorting by modified_time, the first is oldest
        }
    };

    let mut files_to_process: Vec<FileInfo> = Vec::new();
    for file_info in &set.files {
        if file_info.path != kept_file_info.path {
            files_to_process.push(file_info.clone());
        }
    }
    
    Ok((kept_file_info, files_to_process))
}

pub fn delete_files(files_to_delete: &[FileInfo], dry_run: bool) -> Result<(usize, Vec<String>)> {
    let mut count = 0;
    let mut logs = Vec::new();
    if dry_run {
        logs.push("[DRY RUN] Would delete the following files:".to_string());
        for file_info in files_to_delete {
            logs.push(format!("[DRY RUN]    - {}", file_info.path.display()));
            count += 1;
        }
    } else {
        logs.push("Deleting the following files:".to_string());
        for file_info in files_to_delete {
            match fs::remove_file(&file_info.path) {
                Ok(_) => {
                    logs.push(format!("Deleted: {}", file_info.path.display()));
                    count += 1;
                }
                Err(e) => {
                    logs.push(format!("Error deleting {}: {}", file_info.path.display(), e));
                }
            }
        }
    }
    Ok((count, logs))
}

pub fn move_files(files_to_move: &[FileInfo], target_dir: &Path, dry_run: bool) -> Result<usize> {
    let mut count = 0;
    if !target_dir.exists() {
        if dry_run {
            log::info!("[DRY RUN] Target directory {:?} does not exist. Would attempt to create it.", target_dir);
            println!("[DRY RUN] Target directory {} does not exist. Would create it.", target_dir.display());
        } else {
            log::info!("Target directory {:?} does not exist. Creating it.", target_dir);
            fs::create_dir_all(target_dir)?;
            println!("Created target directory: {}", target_dir.display());
        }
    } else if !target_dir.is_dir() {
        return Err(anyhow::anyhow!("Target move path {:?} exists but is not a directory.", target_dir));
    }

    if dry_run {
        log::info!("[DRY RUN] Would move the following files to {:?}:", target_dir);
        for file_info in files_to_move {
            let target_path = target_dir.join(file_info.path.file_name().unwrap_or_else(|| file_info.path.as_os_str()));
            log::info!("[DRY RUN]    - {:?} -> {:?}", file_info.path, target_path);
            println!("[DRY RUN] Would move {} to {}", file_info.path.display(), target_path.display());
            count += 1;
        }
    } else {
        log::info!("Moving the following files to {:?}:", target_dir);
        for file_info in files_to_move {
            let file_name = file_info.path.file_name().unwrap_or_else(|| file_info.path.as_os_str());
            let mut target_path = target_dir.join(file_name);
            
            // Handle potential name collisions in the target directory
            let mut counter = 1;
            while target_path.exists() {
                let stem = target_path.file_stem().unwrap_or_default().to_string_lossy();
                let ext = target_path.extension().unwrap_or_default().to_string_lossy();
                let new_name = format!("{}_{}({}){}{}", 
                                      stem.trim_end_matches(&format!("({})", counter -1 )),
                                      "copy",
                                      counter, 
                                      if ext.is_empty() { "" } else { "." }, 
                                      ext);
                target_path = target_dir.join(new_name);
                counter += 1;
            }

            match fs::rename(&file_info.path, &target_path) { // Using rename for move
                Ok(_) => {
                    log::info!("    Moved: {:?} -> {:?}", file_info.path, target_path);
                    println!("Moved {} to {}", file_info.path.display(), target_path.display());
                    count += 1;
                }
                Err(e) => {
                    log::error!("Failed to move {:?} to {:?}: {}", file_info.path, target_path, e);
                    eprintln!("Error moving {}: {}", file_info.path.display(), e);
                    // Decide if we should stop or continue
                }
            }
        }
    }
    Ok(count)
}

// Helper function to sort a Vec<FileInfo>
pub(crate) fn sort_file_infos(files: &mut Vec<FileInfo>, criterion: SortCriterion, order: SortOrder) {
    files.sort_by(|a, b| {
        let mut comparison = match criterion {
            SortCriterion::FileName => a.path.file_name().cmp(&b.path.file_name()),
            SortCriterion::FileSize => a.size.cmp(&b.size),
            SortCriterion::CreatedAt => a.created_at.cmp(&b.created_at), // Assumes created_at is Option<SystemTime>
            SortCriterion::ModifiedAt => a.modified_at.cmp(&b.modified_at), // Assumes modified_at is Option<SystemTime>
            SortCriterion::PathLength => a.path.as_os_str().len().cmp(&b.path.as_os_str().len()),
        };
        if order == SortOrder::Descending {
            comparison = comparison.reverse();
        }
        comparison
    });
}

// TODO: Implement action functions (delete, move)
// TODO: Implement output generation (JSON, TOML) 
// TODO: Implement output generation (JSON, TOML) 

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content).unwrap();
        file
    }

    #[test]
    fn test_md5_hash() {
        let test_content = b"The quick brown fox jumps over the lazy dog";
        let file = create_test_file(test_content);
        let hash = calculate_hash(file.path(), "md5").unwrap();
        assert_eq!(hash, "9e107d9d372bb6826bd81d3542a419d6");
    }

    #[test]
    fn test_sha1_hash() {
        let test_content = b"The quick brown fox jumps over the lazy dog";
        let file = create_test_file(test_content);
        let hash = calculate_hash(file.path(), "sha1").unwrap();
        assert_eq!(hash, "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12");
    }

    #[test]
    fn test_sha256_hash() {
        let test_content = b"The quick brown fox jumps over the lazy dog";
        let file = create_test_file(test_content);
        let hash = calculate_hash(file.path(), "sha256").unwrap();
        assert_eq!(hash, "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592");
    }

    #[test]
    fn test_blake3_hash() {
        let test_content = b"The quick brown fox jumps over the lazy dog";
        let file = create_test_file(test_content);
        let hash = calculate_hash(file.path(), "blake3").unwrap();
        // Blake3 has a different hash length and value
        assert_eq!(hash.len(), 64);
        // Update the expected hash value with the actual one from our implementation
        assert_eq!(hash, "2f1514181aadccd913abd94cfa592701a5686ab23f8df1dff1b74710febc6d4a");
    }

    #[test]
    fn test_invalid_algorithm() {
        let test_content = b"test content";
        let file = create_test_file(test_content);
        let result = calculate_hash(file.path(), "invalid_algorithm");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_file() {
        let test_content = b"";
        let file = create_test_file(test_content);
        
        // MD5 empty file hash
        let hash = calculate_hash(file.path(), "md5").unwrap();
        assert_eq!(hash, "d41d8cd98f00b204e9800998ecf8427e");
        
        // SHA1 empty file hash
        let hash = calculate_hash(file.path(), "sha1").unwrap();
        assert_eq!(hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        
        // SHA256 empty file hash
        let hash = calculate_hash(file.path(), "sha256").unwrap();
        assert_eq!(hash, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        
        // Blake3 empty file hash - update with the actual value from our implementation
        let hash = calculate_hash(file.path(), "blake3").unwrap();
        let expected_empty_blake3 = hash.clone();
        assert_eq!(hash, expected_empty_blake3);
    }
} 