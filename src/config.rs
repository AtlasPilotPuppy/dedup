use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::cell::RefCell;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::media_dedup::MediaDedupOptions;
use crate::options::DedupOptions;

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
    
    /// Protocol options
    #[cfg(feature = "proto")]
    #[serde(default)]
    pub protocol: ProtocolConfig,
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

    /// Whether to use SSH tunneling for reliable JSON streaming
    #[serde(default = "default_use_ssh_tunnel")]
    pub use_ssh_tunnel: bool,

    /// Default SSH options
    #[serde(default)]
    pub ssh_options: Vec<String>,

    /// Default Rsync options
    #[serde(default)]
    pub rsync_options: Vec<String>,
}

/// Configuration for Protocol options
#[cfg(feature = "proto")]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProtocolConfig {
    /// Whether to use Protobuf instead of JSON
    #[serde(default = "default_use_protobuf")]
    pub use_protobuf: bool,
    
    /// Whether to use ZSTD compression
    #[serde(default = "default_use_compression")]
    pub use_compression: bool,
    
    /// ZSTD compression level (1-22)
    #[serde(default = "default_compression_level")]
    pub compression_level: u32,
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
fn default_use_ssh_tunnel() -> bool {
    true
}

#[cfg(feature = "proto")]
fn default_use_protobuf() -> bool {
    true
}

#[cfg(feature = "proto")]
fn default_use_compression() -> bool {
    true
}

#[cfg(feature = "proto")]
fn default_compression_level() -> u32 {
    3
}

#[cfg(feature = "ssh")]
impl Default for SshConfig {
    fn default() -> Self {
        Self {
            allow_remote_install: default_allow_remote_install(),
            use_remote_dedups: default_use_remote_dedups(),
            use_sudo: default_use_sudo(),
            use_ssh_tunnel: default_use_ssh_tunnel(),
            ssh_options: Vec::new(),
            rsync_options: Vec::new(),
        }
    }
}

#[cfg(feature = "proto")]
impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            use_protobuf: default_use_protobuf(),
            use_compression: default_use_compression(),
            compression_level: default_compression_level(),
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
            #[cfg(feature = "proto")]
            protocol: ProtocolConfig::default(),
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
    
    /// Convert to unified DedupOptions structure
    pub fn to_options(&self) -> DedupOptions {
        // Convert ResolutionPreference to string
        let resolution = match self.media_dedup.resolution_preference {
            crate::media_dedup::ResolutionPreference::Highest => "highest".to_string(),
            crate::media_dedup::ResolutionPreference::Lowest => "lowest".to_string(),
            crate::media_dedup::ResolutionPreference::ClosestTo(w, h) => format!("{}x{}", w, h),
        };
        
        let formats = self.media_dedup.format_preference.formats.clone();
        
        DedupOptions {
            // Basic options
            algorithm: self.algorithm.clone(),
            parallel: self.parallel,
            mode: self.mode.clone(),
            format: self.format.clone(),
            json: self.json,
            progress: self.progress,
            sort_by: self.sort_by.clone(),
            sort_order: self.sort_order.clone(),
            include: self.include.clone(),
            exclude: self.exclude.clone(),
            cache_location: self.cache_location.clone(),
            fast_mode: self.fast_mode,
            
            // Media options
            media_dedup_options: self.media_dedup.clone(),
            media_mode: self.media_dedup.enabled,
            media_resolution: resolution,
            media_formats: formats,
            media_similarity: self.media_dedup.similarity_threshold,
            
            // Set defaults for the rest
            log: false,
            log_file: None,
            output: None,
            interactive: false,
            verbose: 0,
            filter_from: None,
            progress_tui: false,
            raw_sizes: false,
            config_file: None,
            dry_run: false,
            
            // SSH options
            #[cfg(feature = "ssh")]
            allow_remote_install: self.ssh.allow_remote_install,
            #[cfg(feature = "ssh")]
            ssh_options: self.ssh.ssh_options.clone(),
            #[cfg(feature = "ssh")]
            rsync_options: self.ssh.rsync_options.clone(),
            #[cfg(feature = "ssh")]
            use_remote_dedups: self.ssh.use_remote_dedups,
            #[cfg(feature = "ssh")]
            use_sudo: self.ssh.use_sudo,
            #[cfg(feature = "ssh")]
            use_ssh_tunnel: self.ssh.use_ssh_tunnel,
            #[cfg(feature = "ssh")]
            server_mode: false,
            #[cfg(feature = "ssh")]
            port: 0,
            
            // Protocol options
            #[cfg(feature = "proto")]
            use_protobuf: self.protocol.use_protobuf,
            #[cfg(feature = "proto")]
            use_compression: self.protocol.use_compression,
            #[cfg(feature = "proto")]
            compression_level: self.protocol.compression_level,
            
            // Initialize the rest from defaults
            directories: Vec::new(),
            target: None,
            deduplicate: false,
            delete: false,
            move_to: None,
        }
    }
    
    /// Create a config from DedupOptions
    pub fn from_options(options: &DedupOptions) -> Self {
        Self {
            // Basic options
            algorithm: options.algorithm.clone(),
            parallel: options.parallel,
            mode: options.mode.clone(),
            format: options.format.clone(),
            json: options.json,
            progress: options.progress,
            sort_by: options.sort_by.clone(),
            sort_order: options.sort_order.clone(),
            include: options.include.clone(),
            exclude: options.exclude.clone(),
            cache_location: options.cache_location.clone(),
            fast_mode: options.fast_mode,
            
            // Media options
            media_dedup: options.media_dedup_options.clone(),
            
            // SSH options
            #[cfg(feature = "ssh")]
            ssh: SshConfig {
                allow_remote_install: options.allow_remote_install,
                ssh_options: options.ssh_options.clone(),
                rsync_options: options.rsync_options.clone(),
                use_remote_dedups: options.use_remote_dedups,
                use_sudo: options.use_sudo,
                use_ssh_tunnel: options.use_ssh_tunnel,
            },
            
            // Protocol options
            #[cfg(feature = "proto")]
            protocol: ProtocolConfig {
                use_protobuf: options.use_protobuf,
                use_compression: options.use_compression,
                compression_level: options.compression_level,
            },
        }
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
        
        #[cfg(feature = "proto")]
        {
            assert!(config.protocol.use_protobuf);
            assert!(config.protocol.use_compression);
            assert_eq!(config.protocol.compression_level, 3);
        }
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
        
        #[cfg(feature = "proto")]
        {
            test_config.protocol.use_protobuf = false;
            test_config.protocol.compression_level = 9;
        }

        // Save the configuration
        test_config.save_to_path(&config_path)?;

        // Load the configuration back
        let loaded_config = DedupConfig::load_from_path(&config_path)?;

        // Verify loaded config matches saved config
        assert_eq!(loaded_config.algorithm, "sha256");
        assert_eq!(loaded_config.parallel, Some(4));
        assert_eq!(loaded_config.include, vec!["*.jpg", "*.png"]);
        assert_eq!(loaded_config.exclude, vec!["*tmp*"]);
        
        #[cfg(feature = "proto")]
        {
            assert_eq!(loaded_config.protocol.use_protobuf, false);
            assert_eq!(loaded_config.protocol.compression_level, 9);
        }

        Ok(())
    }
    
    #[test]
    fn test_to_options_and_back() -> Result<()> {
        // Create a test configuration
        let mut test_config = DedupConfig::default();
        test_config.algorithm = "sha256".to_string();
        test_config.parallel = Some(4);
        test_config.include = vec!["*.jpg".to_string(), "*.png".to_string()];
        test_config.exclude = vec!["*tmp*".to_string()];
        
        // Convert to options
        let options = test_config.to_options();
        
        // Verify options match config
        assert_eq!(options.algorithm, "sha256");
        assert_eq!(options.parallel, Some(4));
        assert_eq!(options.include, vec!["*.jpg", "*.png"]);
        assert_eq!(options.exclude, vec!["*tmp*"]);
        
        // Convert back to config
        let converted_config = DedupConfig::from_options(&options);
        
        // Verify converted config matches original
        assert_eq!(converted_config.algorithm, "sha256");
        assert_eq!(converted_config.parallel, Some(4));
        assert_eq!(converted_config.include, vec!["*.jpg", "*.png"]);
        assert_eq!(converted_config.exclude, vec!["*tmp*"]);
        
        Ok(())
    }
}
