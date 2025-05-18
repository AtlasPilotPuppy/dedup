#![cfg_attr(feature = "test_mode", allow(unused_imports))]

use anyhow::Result;
use dedups::config::DedupConfig;
use dedups::options::DedupOptions;
use dedups::Cli;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_config_defaults() {
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
fn test_save_and_load_config() -> anyhow::Result<()> {
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let config_path = temp_dir.path().join("test_deduprc");

    // Create a test configuration using direct initialization
    let config = DedupConfig {
        algorithm: "sha256".to_string(),
        parallel: Some(4),
        include: vec!["*.jpg".to_string(), "*.png".to_string()],
        exclude: vec!["*tmp*".to_string()],
        ..Default::default() // Ensure other fields get their defaults
    };

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
    assert_eq!(config.algorithm, "xxhash");
    assert_eq!(config.mode, "newest_modified");

    Ok(())
}

#[cfg(feature = "test_mode")]
#[test]
fn test_create_default_if_not_exists() -> anyhow::Result<()> {
    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let temp_path = temp_dir.path().to_string_lossy().to_string();

    println!("Test temp dir: {}", temp_path);

    // Set up environment for cross-platform testing
    std::env::set_var("HOME", &temp_path);
    std::env::set_var("USERPROFILE", &temp_path);

    // Create a specific config file path inside our temp dir
    let test_config_path = temp_dir.path().join(".deduprc");
    println!("Test config path: {:?}", test_config_path);

    // Force the use of our specific path for this test
    dedups::config::set_test_config_path(Some(test_config_path.clone()));

    // Double-check what config path we'll be using
    let config_path_before = DedupConfig::get_config_path()?;
    println!("Expected config path: {:?}", config_path_before);
    println!(
        "Config path exists before test: {}",
        config_path_before.exists()
    );

    // Create default config
    let create_result = DedupConfig::create_default_if_not_exists();
    println!("Create result: {:?}", create_result);

    // The function should succeed and return true (file was created)
    assert!(
        create_result.is_ok(),
        "Config creation failed: {:?}",
        create_result
    );

    let created = create_result.unwrap();
    println!("Config file was created: {}", created);
    assert!(
        created,
        "Config file already existed or couldn't be created"
    );

    // Get the expected config path and verify the file exists
    let config_path = DedupConfig::get_config_path()?;
    println!("Config path after creation: {:?}", config_path);
    println!("Config path exists after test: {}", config_path.exists());

    assert!(
        config_path.exists(),
        "Config file was not created at expected path: {:?}",
        config_path
    );

    // Second call should return false (file already exists)
    let second_result = DedupConfig::create_default_if_not_exists();
    println!("Second create result: {:?}", second_result);
    assert!(
        second_result.is_ok(),
        "Second config check failed: {:?}",
        second_result
    );
    assert!(
        !second_result.unwrap(),
        "Config file was unexpectedly created twice"
    );

    // Reset the test config path to not affect other tests
    dedups::config::set_test_config_path(None);

    Ok(())
}

#[test]
fn test_custom_config_file() -> anyhow::Result<()> {
    use clap::Parser;

    // Create a temporary directory for testing
    let temp_dir = tempdir()?;
    let custom_config_path = temp_dir.path().join("custom_config.toml");

    // Create a custom configuration using direct initialization
    let custom_config = DedupConfig {
        algorithm: "sha1".to_string(),
        parallel: Some(2),
        progress: true,       // Assuming DedupConfig has this field from its Default
        ..Default::default()  // Ensure other fields get their defaults
    };

    // Save the custom config
    custom_config.save_to_path(&custom_config_path)?;

    // Set up CLI args to use the custom config
    let args = vec![
        "dedup",                              // Program name
        "--interactive",                      // Interactive mode (no directory required)
        "--config-file",                      // Custom config flag
        custom_config_path.to_str().unwrap(), // Custom config path
    ];

    // Parse CLI with our custom args
    let cli = Cli::try_parse_from(args)?;

    // Check that config-file is set correctly
    assert!(cli.config_file.is_some());
    assert_eq!(
        cli.config_file.unwrap().to_string_lossy(),
        custom_config_path.to_string_lossy()
    );

    // Test the config loading directly rather than using with_config(),
    // since we can't safely modify process args in a test
    let config = DedupConfig::load_from_path(&custom_config_path)?;

    // Verify config values from the custom file
    assert_eq!(config.algorithm, "sha1");
    assert_eq!(config.parallel, Some(2));
    assert!(config.progress);

    Ok(())
}

#[test]
fn test_load_config_from_cli_options() {
    // Create a DedupOptions instance that might come from CLI parsing
    let cli_options = DedupOptions {
        algorithm: "sha256".to_string(),
        parallel: Some(4),
        include: vec!["*.jpg".to_string(), "*.png".to_string()],
        exclude: vec!["*tmp*".to_string()],
        // Set other DedupOptions fields as needed for the test
        ..Default::default()
    };

    // Convert DedupOptions to DedupConfig (this is the action under test)
    let config = DedupConfig::from_options(&cli_options);

    // Assertions to verify that DedupConfig is correctly populated
    assert_eq!(config.algorithm, "sha256");
    assert_eq!(config.parallel, Some(4));
    assert_eq!(
        config.include,
        vec!["*.jpg".to_string(), "*.png".to_string()]
    );
    assert_eq!(config.exclude, vec!["*tmp*".to_string()]);
}

#[test]
fn test_custom_config_values_override_defaults() {
    // Create a DedupOptions struct with some custom values
    let custom_options = DedupOptions {
        algorithm: "sha1".to_string(),
        parallel: Some(2),
        progress: true, // This field exists in DedupOptions, ensure it's mapped if DedupConfig has it
        // any other fields for DedupOptions
        ..Default::default()
    };

    // Convert to DedupConfig
    let custom_config = DedupConfig::from_options(&custom_options);

    // Verify that the custom values are reflected in DedupConfig
    assert_eq!(custom_config.algorithm, "sha1");
    assert_eq!(custom_config.parallel, Some(2));
    assert!(custom_config.progress);
}

#[test]
fn test_create_default_config_if_not_exists() -> Result<()> {
    // Create a temporary directory for the test
    let temp_dir = tempdir().unwrap();
    let test_config_dir = temp_dir.path().join("test_config_home");
    fs::create_dir_all(&test_config_dir)?;

    // Set the test config path using the helper if the feature is enabled
    #[cfg(feature = "test_mode")]
    crate::config::set_test_config_path(Some(test_config_dir.join(".deduprc")));

    // Ensure no config file exists initially
    let config_path = DedupConfig::get_config_path()?;
    if config_path.exists() {
        fs::remove_file(&config_path)?;
    }

    // Call the function to create a default config
    let created = DedupConfig::create_default_if_not_exists()?;
    assert!(created, "Config file should have been created");
    assert!(
        config_path.exists(),
        "Config file should exist at the expected path"
    );

    // Verify the content of the created config file (optional, basic check)
    let loaded_config = DedupConfig::load_from_path(&config_path)?;
    assert_eq!(loaded_config.algorithm, "xxhash".to_string());

    // Call again, should not create
    let created_again = DedupConfig::create_default_if_not_exists()?;
    assert!(!created_again, "Config file should not be created again");

    // Clean up: Reset test config path and remove the created file
    #[cfg(feature = "test_mode")]
    crate::config::set_test_config_path(None);
    if config_path.exists() {
        fs::remove_file(config_path)?;
    }
    Ok(())
}

// Placeholder for the actual test function that needs fixing near line 30
#[test]
fn test_config_initialization_fix_1() {
    // Renamed for clarity if this is a new test stub
    let config = DedupConfig {
        algorithm: "sha256".to_string(),
        parallel: Some(4),
        include: vec!["*.jpg".to_string(), "*.png".to_string()],
        exclude: vec!["*tmp*".to_string()],
        // Add other fields that were being set if necessary
        ..Default::default()
    };
    // Add assertions relevant to this test scenario
    assert_eq!(config.algorithm, "sha256");
}
