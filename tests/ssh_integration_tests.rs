#![cfg(feature = "ssh")]

use anyhow::Result;
use std::path::Path;
use dedups::file_utils;
use dedups::ssh_utils::RemoteLocation;
use dedups::Cli;
use clap::Parser;
use dedups::ssh_utils::SshProtocol;

/// Integration tests for SSH functionality using a local host for testing
#[test]
fn test_ssh_remote_basic_operations() -> Result<()> {
    // Test basic connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    if ensure_ssh_access(&remote).is_err() { return Ok(()); }
    
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
fn test_is_remote_path() -> Result<()> {
    // Test path detection
    assert!(file_utils::is_remote_path(Path::new("ssh:testuser@localhost:2222:/tmp")));
    assert!(!file_utils::is_remote_path(Path::new("/tmp")));
    
    Ok(())
}

#[test]
fn test_check_dedups_installed() -> Result<()> {
    // Test checking if dedups is installed on remote
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    
    // Create tokio runtime for async functions
    let rt = tokio::runtime::Runtime::new()?;
    
    // Check if dedups is installed
    let is_installed = rt.block_on(remote.check_dedups_installed())?;
    
    // Just print the result, don't assert since we don't know if it's installed
    println!("Is dedups installed on Docker test container: {:?}", is_installed);
    
    Ok(())
}

#[test]
fn test_file_operations() -> Result<()> {
    // Create a test CLI configuration
    if RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp").and_then(|r| ensure_ssh_access(&r)).is_err() { return Ok(()); }
    let _cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install", "/tmp"]);
    
    // Create a temporary local file
    let local_tempdir = tempfile::tempdir()?;
    let local_file = local_tempdir.path().join("local_test_file.txt");
    std::fs::write(&local_file, "Test content for SSH operations")?;
    
    // Create a remote path
    let remote_path_str = format!("ssh:testuser@localhost:2222:/tmp/dedups_remote_test_file.txt");
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
fn test_handle_directory() -> Result<()> {
    // Skip if no SSH
    if RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp").and_then(|r| ensure_ssh_access(&r)).is_err() { return Ok(()); }
    // Create a minimal CLI configuration
    let mut cli = Cli::parse_from(["dedups", "ssh:testuser@localhost:2222:/home/testuser/test_data"]);
    
    // Override any config that might be set
    cli.allow_remote_install = false; // Don't try to install dedups for this test
    
    // Create some test files in the remote directory
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/home/testuser/test_data")?;
    remote.run_command("mkdir -p /home/testuser/test_data && touch /home/testuser/test_data/test1.txt /home/testuser/test_data/test2.txt")?;
    
    // Try to scan the remote directory
    let files = file_utils::handle_directory(&cli, Path::new("ssh:testuser@localhost:2222:/home/testuser/test_data"))?;
    
    // Print some info about the files found
    println!("Found {} files in remote test directory", files.len());
    for (i, file) in files.iter().take(5).enumerate() {
        println!("  File {}: {}", i + 1, file.path.display());
    }
    
    // We should have found at least some files
    assert!(!files.is_empty(), "No files found in remote test directory");
    
    Ok(())
}

#[test]
fn test_remote_dedups_execution() -> Result<()> {
    // Skip if no SSH
    if RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp").and_then(|r| ensure_ssh_access(&r)).is_err() { return Ok(()); }
    // Create a test CLI configuration
    let cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install", "/tmp"]);
    
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    
    // First check if dedups is installed
    let rt = tokio::runtime::Runtime::new()?;
    let dedups_path = rt.block_on(remote.check_dedups_installed())?;
    
    // Create a mock dedups script if not installed
    if dedups_path.is_none() {
        println!("Creating mock dedups script...");
        remote.run_command(r#"
            mkdir -p ~/.local/bin
            cat > ~/.local/bin/dedups << 'EOF'
#!/bin/bash
# Mock dedups script for testing
if [[ "$1" == "--version" ]]; then
  echo "dedups v0.1.0-test"
  exit 0
fi
echo "Mock dedups running with args: $@"
echo "Base directory: $1"
echo "Found 2 duplicate sets with 4 files"
exit 0
EOF
            chmod +x ~/.local/bin/dedups
        "#)?;
    }
    
    // Set up SSH protocol
    let mut protocol = SshProtocol::new(remote);
    protocol.connect()?;
    
    // Try to execute a simple dedups command
    let output = protocol.execute_dedups(&["--version"], &cli)?;
    assert!(!output.is_empty(), "dedups --version should return version info");
    
    // Try to scan a directory
    let scan_output = protocol.execute_dedups(&["/tmp", "--dry-run"], &cli)?;
    assert!(!scan_output.is_empty(), "dedups scan should return some output");
    
    Ok(())
}

#[test]
fn test_remote_dedups_path_handling() -> Result<()> {
    // Skip if no SSH
    if RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp").and_then(|r| ensure_ssh_access(&r)).is_err() { return Ok(()); }
    // Create a test CLI configuration
    let cli = Cli::parse_from(&["dedups", "--use-sudo", "--allow-remote-install", "/tmp"]);
    
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    
    // Create a mock dedups script if not found
    remote.run_command(r#"
        mkdir -p ~/.local/bin
        cat > ~/.local/bin/dedups << 'EOF'
#!/bin/bash
# Mock dedups script for testing
if [[ "$1" == "--version" ]]; then
  echo "dedups v0.1.0-test"
  exit 0
fi
echo "Mock dedups running with args: $@"
exit 0
EOF
        chmod +x ~/.local/bin/dedups
    "#)?;
    
    // Verify dedups is found
    let rt = tokio::runtime::Runtime::new()?;
    let dedups_path = rt.block_on(remote.check_dedups_installed())?;
    assert!(dedups_path.is_some(), "dedups should be found in ~/.local/bin");
    
    // Try to execute dedups through SshProtocol
    let mut protocol = SshProtocol::new(remote.clone());
    protocol.connect()?;
    
    let output = protocol.execute_dedups(&["--version"], &cli)?;
    assert!(output.contains("dedups v0.1.0-test"), "Should execute the test dedups script");
    
    Ok(())
}

// Helper to check SSH connectivity and skip tests when SSH not available
fn ensure_ssh_access(remote: &RemoteLocation) -> Result<()> {
    match remote.run_command("echo connectivity_test") {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Skipping SSH test due to connectivity issue: {}", e);
            Err(anyhow::anyhow!("SKIP"))
        }
    }
} 