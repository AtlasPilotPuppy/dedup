#![cfg(feature = "ssh")]

use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use dedups::file_utils;
use dedups::ssh_utils::{RemoteLocation, SshProtocol};
use dedups::Cli;
use std::time::Instant;

/// Test JSON streaming over SSH with and without tunneling
#[test]
fn test_ssh_json_streaming() -> Result<()> {
    // Create a test CLI configuration with JSON output enabled
    let mut cli = Cli::parse_from(&["dedups", "--use-sudo", "--json", "/tmp"]);

    // First test with tunneling enabled (default)
    cli.use_ssh_tunnel = true;

    // Set up remote connection to the Docker test container
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    if ensure_ssh_access(&remote).is_err() {
        return Ok(());
    }

    // Set up the SSH protocol
    let mut protocol = SshProtocol::new(remote.clone());
    protocol.connect()?;

    // Measure tunnel performance
    let start = Instant::now();
    println!("Testing JSON streaming WITH tunnel...");
    let output_with_tunnel = protocol.execute_dedups(&["--json"], &cli)?;
    let tunnel_duration = start.elapsed();

    // Verify JSON output with tunnel
    assert!(
        output_with_tunnel.contains("\"type\":"),
        "Output should contain JSON data"
    );

    // Now test without tunneling
    cli.use_ssh_tunnel = false;

    // Measure standard SSH performance
    let start = Instant::now();
    println!("Testing JSON streaming WITHOUT tunnel...");
    let output_without_tunnel = protocol.execute_dedups(&["--json"], &cli)?;
    let standard_duration = start.elapsed();

    // Verify JSON output without tunnel
    assert!(
        output_without_tunnel.contains("\"type\":"),
        "Output should contain JSON data"
    );

    // Print performance comparison
    println!("JSON streaming WITH tunnel took: {:?}", tunnel_duration);
    println!(
        "JSON streaming WITHOUT tunnel took: {:?}",
        standard_duration
    );

    Ok(())
}

/// Test netcat availability on remote system (required for tunneling)
#[test]
fn test_netcat_availability() -> Result<()> {
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    if ensure_ssh_access(&remote).is_err() {
        return Ok(());
    }

    // Check if netcat is available
    let nc_check = remote.run_command("command -v nc || command -v netcat")?;

    // Output should contain path to netcat
    assert!(
        !nc_check.trim().is_empty(),
        "Netcat should be available on the remote system"
    );
    println!("Netcat found at: {}", nc_check.trim());

    Ok(())
}

/// Test stdbuf availability on remote system (fallback for unbuffered output)
#[test]
fn test_stdbuf_availability() -> Result<()> {
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    if ensure_ssh_access(&remote).is_err() {
        return Ok(());
    }

    // Check if stdbuf is available
    let stdbuf_check = remote.run_command("command -v stdbuf")?;

    // Output should contain path to stdbuf
    assert!(
        !stdbuf_check.trim().is_empty(),
        "stdbuf should be available on the remote system"
    );
    println!("stdbuf found at: {}", stdbuf_check.trim());

    Ok(())
}

/// Test creating a mock dedups script on the remote system and executing it with JSON output
#[test]
fn test_remote_dedups_json_execution() -> Result<()> {
    // Set up remote connection
    let remote = RemoteLocation::parse("ssh:testuser@localhost:2222:/tmp")?;
    if ensure_ssh_access(&remote).is_err() {
        return Ok(());
    }

    // Create a mock dedups script
    let create_script_cmd = r#"
        mkdir -p ~/.local/bin
        cat > ~/.local/bin/dedups << 'EOF'
#!/bin/bash
# Mock dedups script for testing JSON streaming

if [[ "$1" == "--version" ]]; then
  echo "dedups v0.1.0-test"
  exit 0
fi

if [[ "$1" == "--json" ]]; then
  # Output structured JSON for testing JSON streaming
  echo "{\"type\":\"progress\",\"stage\":1,\"stage_name\":\"Scanning\",\"files_processed\":0,\"total_files\":10,\"percent_complete\":0,\"bytes_processed\":0,\"total_bytes\":0,\"elapsed_seconds\":0.1,\"status_message\":\"Starting scan...\"}"
  sleep 0.5
  echo "{\"type\":\"progress\",\"stage\":1,\"stage_name\":\"Scanning\",\"files_processed\":5,\"total_files\":10,\"percent_complete\":50,\"bytes_processed\":1024,\"total_bytes\":2048,\"elapsed_seconds\":0.5,\"status_message\":\"Scanning in progress...\"}"
  sleep 0.5
  echo "{\"type\":\"progress\",\"stage\":1,\"stage_name\":\"Scanning\",\"files_processed\":10,\"total_files\":10,\"percent_complete\":100,\"bytes_processed\":2048,\"total_bytes\":2048,\"elapsed_seconds\":1.0,\"status_message\":\"Scan complete\"}"
  sleep 0.5
  echo "{\"type\":\"result\",\"duplicate_count\":2,\"total_files\":10,\"total_bytes\":2048,\"duplicate_bytes\":1024,\"elapsed_seconds\":1.5}"
  exit 0
fi

echo "Mock dedups running with args: $@"
echo "Base directory: $1"
echo "Found 2 duplicate sets with 4 files"
exit 0
EOF
        chmod +x ~/.local/bin/dedups
    "#;

    let script_result = remote.run_command(create_script_cmd)?;
    println!("Script creation result: {}", script_result);

    // Verify the script exists and is executable
    let check_script =
        remote.run_command("test -x ~/.local/bin/dedups && echo 'exists' || echo 'missing'")?;
    assert_eq!(
        check_script.trim(),
        "exists",
        "dedups script should exist and be executable"
    );

    // Create CLI configuration
    let mut cli = Cli::parse_from(&["dedups", "--json", "/tmp"]);

    // Test with tunnel enabled
    cli.use_ssh_tunnel = true;

    // Set up SSH protocol
    let mut protocol = SshProtocol::new(remote.clone());
    protocol.connect()?;

    // Execute with JSON output
    let output = protocol.execute_dedups(&["--json"], &cli)?;

    // Verify the output contains JSON progress updates
    assert!(
        output.contains("\"type\":\"progress\""),
        "Output should contain progress updates"
    );
    assert!(
        output.contains("\"type\":\"result\""),
        "Output should contain final result"
    );

    // Count the number of JSON objects in the output
    let json_object_count = output
        .lines()
        .filter(|line| line.trim().starts_with("{"))
        .count();
    assert!(
        json_object_count >= 3,
        "Expected at least 3 JSON objects in the output"
    );

    println!(
        "Received {} JSON objects from remote dedups execution",
        json_object_count
    );

    Ok(())
}

fn ensure_ssh_access(remote: &RemoteLocation) -> Result<()> {
    match remote.run_command("echo connectivity_test") {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("Skipping test due to SSH connectivity issue: {}", e);
            Err(anyhow::anyhow!("SKIP"))
        }
    }
}
