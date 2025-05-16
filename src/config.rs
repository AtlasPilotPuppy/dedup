use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::fs;
use std::io::ErrorKind;
use anyhow::{Result, Context};

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
    "modifiedat".to_string()
}

fn default_sort_order() -> String {
    "descending".to_string()
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            algorithm: default_algorithm(),
            parallel: None,
            mode: default_mode(),
            format: default_format(),
            progress: false,
            sort_by: default_sort_by(),
            sort_order: default_sort_order(),
            include: Vec::new(),
            exclude: Vec::new(),
            cache_location: None,
            fast_mode: false,
        }
    }
}

impl DedupConfig {
    /// Get the path to the user's config file
    pub fn get_config_path() -> Result<PathBuf> {
        // Try to get the user's home directory
        let home_dir = dirs::home_dir()
            .context("Could not determine home directory")?;
        
        Ok(home_dir.join(".deduprc"))
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
        let toml = toml::to_string_pretty(self)
            .context("Failed to serialize config to TOML")?;
        
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
        assert_eq!(config.sort_by, "modifiedat");
        assert_eq!(config.sort_order, "descending");
        assert!(config.include.is_empty());
        assert!(config.exclude.is_empty());
        assert_eq!(config.parallel, None);
        assert_eq!(config.progress, false);
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