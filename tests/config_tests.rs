use dedup_tui::config::DedupConfig;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_config_defaults() {
    let config = DedupConfig::default();
    
    assert_eq!(config.algorithm, "blake3");
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
fn test_save_and_load_config() -> anyhow::Result<()> {
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let config_path = temp_dir.path().join("test_deduprc");
    
    // Create a test configuration
    let mut config = DedupConfig::default();
    config.algorithm = "sha256".to_string();
    config.parallel = Some(4);
    config.include = vec!["*.jpg".to_string(), "*.png".to_string()];
    config.exclude = vec!["*tmp*".to_string()];
    
    // Save the configuration
    config.save_to_path(&config_path)?;
    
    // Verify the file exists and has content
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path)?;
    assert!(content.contains("algorithm = \"sha256\""));
    assert!(content.contains("parallel = 4"));
    
    // Load the configuration back
    let loaded_config = DedupConfig::load_from_path(&config_path)?;
    
    // Verify loaded config matches saved config
    assert_eq!(loaded_config.algorithm, "sha256");
    assert_eq!(loaded_config.parallel, Some(4));
    assert_eq!(loaded_config.include, vec!["*.jpg", "*.png"]);
    assert_eq!(loaded_config.exclude, vec!["*tmp*"]);
    
    // Default values should be preserved
    assert_eq!(loaded_config.mode, "newest_modified");
    assert_eq!(loaded_config.format, "json");
    
    Ok(())
}

#[test]
fn test_nonexistent_config() -> anyhow::Result<()> {
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let nonexistent_path = temp_dir.path().join("nonexistent_config");
    
    // Loading a non-existent config should return defaults
    let config = DedupConfig::load_from_path(&nonexistent_path)?;
    
    // Check default values
    assert_eq!(config.algorithm, "blake3");
    assert_eq!(config.mode, "newest_modified");
    
    Ok(())
}

#[test]
fn test_create_default_if_not_exists() -> anyhow::Result<()> {
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let config_path = temp_dir.path().join(".deduprc");
    
    // Set a test home directory to avoid affecting the real user's config
    std::env::set_var("HOME", temp_dir.path());
    
    // Mock the config path function for testing
    let create_result = DedupConfig::create_default_if_not_exists();
    
    // The function should succeed and return true (file was created)
    assert!(create_result.is_ok());
    
    // Check if the file was created
    assert!(config_path.exists());
    
    // If we run it again, it should return false (file already exists)
    let second_result = DedupConfig::create_default_if_not_exists();
    assert!(second_result.is_ok());
    assert!(!second_result.unwrap());
    
    Ok(())
}

#[test]
fn test_custom_config_file() -> anyhow::Result<()> {
    use clap::Parser;
    use dedup_tui::Cli;
    
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let custom_config_path = temp_dir.path().join("custom_config.toml");
    
    // Create a custom configuration
    let mut custom_config = DedupConfig::default();
    custom_config.algorithm = "sha1".to_string();
    custom_config.parallel = Some(2);
    custom_config.progress = true;
    
    // Save the custom config
    custom_config.save_to_path(&custom_config_path)?;
    
    // Set up CLI args to use the custom config
    let args = vec![
        "dedup_tui",           // Program name
        "--interactive",       // Interactive mode (no directory required)
        "--config-file",       // Custom config flag
        custom_config_path.to_str().unwrap(), // Custom config path
    ];
    
    // Parse CLI with our custom args
    let cli = Cli::try_parse_from(args)?;
    
    // Check that config-file is set correctly
    assert!(cli.config_file.is_some());
    assert_eq!(cli.config_file.unwrap().to_string_lossy(), custom_config_path.to_string_lossy());
    
    // Test the config loading directly rather than using with_config(),
    // since we can't safely modify process args in a test
    let config = DedupConfig::load_from_path(&custom_config_path)?;
    
    // Verify config values from the custom file 
    assert_eq!(config.algorithm, "sha1");
    assert_eq!(config.parallel, Some(2));
    assert_eq!(config.progress, true);
    
    Ok(())
} 