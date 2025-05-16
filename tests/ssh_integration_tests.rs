#![cfg(feature = "ssh")]

use anyhow::Result;
use std::path::Path;
use dedups::file_utils;
use dedups::ssh_utils::RemoteLocation;
use dedups::Cli;
use clap::Parser;
use dedups::ssh_utils::SshProtocol;

/// Integration tests for SSH functionality using a local host for testing
/// Note: These tests require a 'local' host configured in ~/.ssh/config
#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_ssh_remote_basic_operations() -> Result<()> {
    // Test basic connection
    let remote = RemoteLocation::parse("ssh:local:/tmp")?;
    
    // Run a simple command
    let output = remote.run_command("echo 'SSH test successful'")?;
    assert_eq!(output.trim(), "SSH test successful");
    
    // Check if we can write to /tmp
    let write_test = remote.run_command("touch /tmp/dedups_test_file && echo 'success' || echo 'failed'")?;
    assert!(write_test.contains("success"), "Failed to write to /tmp on remote host");
    
    // Clean up
    let _ = remote.run_command("rm -f /tmp/dedups_test_file");
    
    Ok(())
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_is_remote_path() -> Result<()> {
    // Test path detection
    assert!(file_utils::is_remote_path(Path::new("ssh:local:/tmp")));
    assert!(!file_utils::is_remote_path(Path::new("/tmp")));
    
    Ok(())
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_check_dedups_installed() -> Result<()> {
    // Test checking if dedups is installed on remote
    let remote = RemoteLocation::parse("ssh:local:/tmp")?;
    
    // Create tokio runtime for async functions
    let rt = tokio::runtime::Runtime::new()?;
    
    // Check if dedups is installed
    let is_installed = rt.block_on(remote.check_dedups_installed())?;
    
    // Just print the result, don't assert since we don't know if it's installed
    println!("Is dedups installed on local test host: {:?}", is_installed);
    
    Ok(())
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_file_operations() -> Result<()> {
    // Create a test CLI configuration
    let _cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install"]);
    
    // Create a temporary local file
    let local_tempdir = tempfile::tempdir()?;
    let local_file = local_tempdir.path().join("local_test_file.txt");
    std::fs::write(&local_file, "Test content for SSH operations")?;
    
    // Create a remote path
    let remote_path_str = format!("ssh:local:/tmp/dedups_remote_test_file.txt");
    let remote_path = Path::new(&remote_path_str);
    
    // Copy the file to remote
    println!("Copying file to remote...");
    file_utils::copy_file(&local_file, remote_path, false)?;
    
    // Verify file exists remotely
    let remote = RemoteLocation::parse(&remote_path_str)?;
    let check_cmd = format!("test -f {} && echo 'exists' || echo 'missing'", remote.path.display());
    let check_result = remote.run_command(&check_cmd)?;
    assert!(check_result.contains("exists"), "Remote file was not created");
    
    // Read the remote file content
    let cat_cmd = format!("cat {}", remote.path.display());
    let content = remote.run_command(&cat_cmd)?;
    assert_eq!(content.trim(), "Test content for SSH operations");
    
    // Delete the remote file
    println!("Deleting remote file...");
    let delete_cmd = format!("rm -f {}", remote.path.display());
    let _ = remote.run_command(&delete_cmd);
    
    // Clean up local temporary directory
    drop(local_tempdir);
    
    Ok(())
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_handle_directory() -> Result<()> {
    // Create a minimal CLI configuration
    let mut cli = Cli::parse_from(["dedups", "ssh:local:/tmp"]);
    
    // Override any config that might be set
    cli.allow_remote_install = false; // Don't try to install dedups for this test
    
    // Try to scan the remote directory
    let files = file_utils::handle_directory(&cli, Path::new("ssh:local:/tmp"))?;
    
    // Print some info about the files found
    println!("Found {} files in remote /tmp directory", files.len());
    for (i, file) in files.iter().take(5).enumerate() {
        println!("  File {}: {}", i + 1, file.path.display());
    }
    
    // We should have found at least some files
    assert!(!files.is_empty(), "No files found in remote /tmp directory");
    
    Ok(())
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_remote_dedups_execution() -> Result<()> {
    // Create a test CLI configuration
    let cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install"]);
    
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:local:/tmp")?;
    
    // First check if dedups is installed
    let rt = tokio::runtime::Runtime::new()?;
    let dedups_path = rt.block_on(remote.check_dedups_installed())?;
    
    match dedups_path {
        Some(path) => {
            log::info!("Found dedups at: {}", path);
            
            // Set up SSH protocol
            let mut protocol = SshProtocol::new(remote);
            protocol.connect()?;
            
            // Try to execute a simple dedups command
            let output = protocol.execute_dedups(&["--version"])?;
            assert!(!output.is_empty(), "dedups --version should return version info");
            
            // Try to scan a directory
            let scan_output = protocol.execute_dedups(&["/tmp", "--dry-run"])?;
            assert!(!scan_output.is_empty(), "dedups scan should return some output");
            
            Ok(())
        },
        None => {
            if cli.allow_remote_install {
                // Try to install dedups
                remote.install_dedups(&cli)?;
                
                // Verify installation
                let dedups_path = rt.block_on(remote.check_dedups_installed())?;
                assert!(dedups_path.is_some(), "dedups should be installed after installation");
                
                // Now try to use it
                let mut protocol = SshProtocol::new(remote);
                protocol.connect()?;
                
                let output = protocol.execute_dedups(&["--version"])?;
                assert!(!output.is_empty(), "dedups --version should return version info");
                
                Ok(())
            } else {
                Ok(()) // Skip test if installation not allowed
            }
        }
    }
}

#[test]
#[ignore] // These tests require a configured SSH host and are ignored by default
fn test_remote_dedups_path_handling() -> Result<()> {
    // Create a test CLI configuration
    let cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install"]);
    
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:local:/tmp")?;
    
    // Create a test file in ~/.local/bin to simulate local installation
    let setup_cmd = r#"
        mkdir -p ~/.local/bin
        echo '#!/bin/bash\necho "test dedups v0.1.0"' > ~/.local/bin/dedups
        chmod +x ~/.local/bin/dedups
    "#;
    remote.run_command(setup_cmd)?;
    
    // Verify dedups is found
    let rt = tokio::runtime::Runtime::new()?;
    let dedups_path = rt.block_on(remote.check_dedups_installed())?;
    assert!(dedups_path.is_some(), "dedups should be found in ~/.local/bin");
    
    // Try to execute dedups through SshProtocol
    let mut protocol = SshProtocol::new(remote.clone());
    protocol.connect()?;
    
    let output = protocol.execute_dedups(&["--version"])?;
    assert!(output.contains("test dedups"), "Should execute the test dedups script");
    
    // Clean up
    remote.run_command("rm -f ~/.local/bin/dedups")?;
    
    Ok(())
} 