use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::cell::RefCell;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::media_dedup::MediaDedupOptions;

// For tests only - enable with test_mode feature
#[cfg(feature = "test_mode")]
thread_local! {
    static TEST_CONFIG_PATH: RefCell<Option<PathBuf>> = RefCell::new(None);
}

// For tests only - helper to set a test config path
#[cfg(feature = "test_mode")]
pub fn set_test_config_path(path: Option<PathBuf>) {
    TEST_CONFIG_PATH.with(|cell| {
        *cell.borrow_mut() = path;
    });
}

/// Configuration structure for .deduprc file
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DedupConfig {
    /// Hashing algorithm to use for comparing files
    #[serde(default = "default_algorithm")]
    pub algorithm: String,

    /// Number of parallel threads to use for hashing
    #[serde(default)]
    pub parallel: Option<usize>,

    /// Selection strategy for delete/move operations
    #[serde(default = "default_mode")]
    pub mode: String,

    /// Default output format
    #[serde(default = "default_format")]
    pub format: String,
    
    /// Whether to output results in JSON format to stdout
    #[serde(default)]
    pub json: bool,

    /// Whether to show progress information during scanning/hashing
    #[serde(default)]
    pub progress: bool,

    /// Default sort criterion
    #[serde(default = "default_sort_by")]
    pub sort_by: String,

    /// Default sort order
    #[serde(default = "default_sort_order")]
    pub sort_order: String,

    /// Default file include patterns
    #[serde(default)]
    pub include: Vec<String>,

    /// Default file exclude patterns
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Location to store file hash cache
    #[serde(default)]
    pub cache_location: Option<PathBuf>,

    /// Whether to use fast mode (use cached hashes for unchanged files)
    #[serde(default)]
    pub fast_mode: bool,

    /// Media deduplication options
    #[serde(default)]
    pub media_dedup: MediaDedupOptions,

    /// SSH remote options
    #[cfg(feature = "ssh")]
    #[serde(default)]
    pub ssh: SshConfig,
}

/// Configuration for SSH remote operations
#[cfg(feature = "ssh")]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SshConfig {
    /// Allow installation of dedups on remote systems
    #[serde(default = "default_allow_remote_install")]
    pub allow_remote_install: bool,

    /// Whether to use remote dedups if available
    #[serde(default = "default_use_remote_dedups")]
    pub use_remote_dedups: bool,

    /// Whether to use sudo for installation (if available)
    #[serde(default = "default_use_sudo")]
    pub use_sudo: bool,

    /// Default SSH options
    #[serde(default)]
    pub ssh_options: Vec<String>,

    /// Default Rsync options
    #[serde(default)]
    pub rsync_options: Vec<String>,
}

fn default_algorithm() -> String {
    "xxhash".to_string()
}

fn default_mode() -> String {
    "newest_modified".to_string()
}

fn default_format() -> String {
    "json".to_string()
}

fn default_sort_by() -> String {
    "modified".to_string()
}

fn default_sort_order() -> String {
    "descending".to_string()
}

#[cfg(feature = "ssh")]
fn default_allow_remote_install() -> bool {
    true
}

#[cfg(feature = "ssh")]
fn default_use_remote_dedups() -> bool {
    true
}

#[cfg(feature = "ssh")]
fn default_use_sudo() -> bool {
    false
}

#[cfg(feature = "ssh")]
impl Default for SshConfig {
    fn default() -> Self {
        Self {
            allow_remote_install: default_allow_remote_install(),
            use_remote_dedups: default_use_remote_dedups(),
            use_sudo: default_use_sudo(),
            ssh_options: Vec::new(),
            rsync_options: Vec::new(),
        }
    }
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            algorithm: default_algorithm(),
            parallel: None,
            mode: default_mode(),
            format: default_format(),
            json: false,
            progress: false,
            sort_by: default_sort_by(),
            sort_order: default_sort_order(),
            include: Vec::new(),
            exclude: Vec::new(),
            cache_location: None,
            fast_mode: false,
            media_dedup: MediaDedupOptions::default(),
            #[cfg(feature = "ssh")]
            ssh: SshConfig::default(),
        }
    }
}

impl DedupConfig {
    /// Get the path to the user's config file
    pub fn get_config_path() -> Result<PathBuf> {
        // For tests, return the test config path if set
        #[cfg(feature = "test_mode")]
        {
            if let Some(path) = TEST_CONFIG_PATH.with(|cell| cell.borrow().clone()) {
                log::debug!("Using test config path: {:?}", path);
                return Ok(path);
            }
        }

        // Normal config path logic
        let path = {
            #[cfg(target_family = "unix")]
            {
                // Unix-style: ~/.deduprc
                let home_dir = dirs::home_dir().context("Could not determine home directory")?;
                log::debug!("Unix config path. Home dir: {:?}", home_dir);
                home_dir.join(".deduprc")
            }

            #[cfg(target_family = "windows")]
            {
                // Windows-style: %APPDATA%\dedup\config.toml or %USERPROFILE%\.deduprc as fallback
                let config_dir = dirs::config_dir();
                log::debug!("Windows config dir: {:?}", config_dir);

                if let Some(config_dir) = config_dir {
                    let target_path = config_dir.join("dedup").join("config.toml");
                    log::debug!("Windows target config path: {:?}", target_path);

                    // Create parent directory if it doesn't exist already
                    if let Some(parent) = target_path.parent() {
                        log::debug!(
                            "Windows config parent dir: {:?}, exists: {}",
                            parent,
                            parent.exists()
                        );
                        if !parent.exists() {
                            log::debug!("Creating parent directory for Windows config");
                            match fs::create_dir_all(parent) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::warn!(
                                        "Failed to create Windows config dir {:?}: {}",
                                        parent,
                                        e
                                    );
                                    // On failure, fall back to home directory
                                    let home_dir = dirs::home_dir()
                                        .context("Could not determine home directory")?;
                                    log::debug!("Falling back to Windows home dir: {:?}", home_dir);
                                    return Ok(home_dir.join(".deduprc"));
                                }
                            }
                        }
                    }
                    target_path
                } else {
                    // Fallback to home directory if config_dir isn't available
                    let home_dir =
                        dirs::home_dir().context("Could not determine home directory")?;
                    log::debug!(
                        "No Windows config dir available, using home: {:?}",
                        home_dir
                    );
                    home_dir.join(".deduprc")
                }
            }
        };

        log::debug!("Final config path: {:?}, exists: {}", path, path.exists());
        Ok(path)
    }

    /// Load configuration from the .deduprc file
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;
        Self::load_from_path(&config_path)
    }

    /// Load configuration from a specific path
    pub fn load_from_path(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                // Parse the TOML content
                let config: DedupConfig = toml::from_str(&contents)
                    .with_context(|| format!("Failed to parse config file: {:?}", path))?;
                Ok(config)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // Config file doesn't exist, return default config
                Ok(Self::default())
            }
            Err(e) => {
                // Other error occurred
                Err(e).with_context(|| format!("Failed to read config file: {:?}", path))
            }
        }
    }

    /// Save the current configuration to the .deduprc file
    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        self.save_to_path(&config_path)
    }

    /// Save the configuration to a specific path
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let toml = toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        fs::write(path, toml)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;

        Ok(())
    }

    /// Create a default configuration file if it doesn't exist
    pub fn create_default_if_not_exists() -> Result<bool> {
        let config_path = Self::get_config_path()?;

        // Check if the config file already exists
        if config_path.exists() {
            return Ok(false); // File already exists, no action taken
        }

        // Create and save a default configuration
        let default_config = Self::default();
        default_config.save_to_path(&config_path)?;

        Ok(true) // File was created
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = DedupConfig::default();
        assert_eq!(config.algorithm, "xxhash");
        assert_eq!(config.mode, "newest_modified");
        assert_eq!(config.format, "json");
        assert_eq!(config.sort_by, "modified");
        assert_eq!(config.sort_order, "descending");
        assert!(config.include.is_empty());
        assert!(config.exclude.is_empty());
        assert_eq!(config.parallel, None);
        assert!(!config.progress);
    }

    #[test]
    fn test_save_and_load_config() -> Result<()> {
        // Create a temporary directory for testing
        let temp_dir = tempdir()?;
        let config_path = temp_dir.path().join("test_config.toml");

        // Create a test configuration
        let mut test_config = DedupConfig::default();
        test_config.algorithm = "sha256".to_string();
        test_config.parallel = Some(4);
        test_config.include = vec!["*.jpg".to_string(), "*.png".to_string()];
        test_config.exclude = vec!["*tmp*".to_string()];

        // Save the configuration
        test_config.save_to_path(&config_path)?;

        // Load the configuration back
        let loaded_config = DedupConfig::load_from_path(&config_path)?;

        // Verify loaded config matches saved config
        assert_eq!(loaded_config.algorithm, "sha256");
        assert_eq!(loaded_config.parallel, Some(4));
        assert_eq!(loaded_config.include, vec!["*.jpg", "*.png"]);
        assert_eq!(loaded_config.exclude, vec!["*tmp*"]);

        Ok(())
    }
}
