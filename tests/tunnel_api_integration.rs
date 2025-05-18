// New integration test for tunnel API client/server handshake
// Requires ssh + proto features and an environment variable enabling it because
// it spawns real child processes and binds to a TCP port.
//
// Run manually with:
//   cargo test --test tunnel_api_integration --features "ssh,proto" \
//      -- --ignored    DEDUPS_SSH_TEST=1
//
// The test starts a local dedups instance in --server-mode and then
// connects to it with DedupClient, issuing an internal handshake.

#![cfg(all(feature = "ssh", feature = "proto"))]

use std::collections::HashMap;
use std::process::{Command, Stdio, Child};
use std::time::{Duration, Instant};
use std::net::TcpStream;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use assert_cmd::cargo::cargo_bin;

use dedups::client::{DedupClient, ConnectionState};
use dedups::protocol::find_available_port;

const SERVER_START_TIMEOUT: u64 = 5000;
const SERVER_STOP_TIMEOUT: u64 = 3000;
const PORT_RANGE_START: u16 = 13000;  // Changed to avoid conflicts
const PORT_RANGE_END: u16 = 14000;
const MAX_PORT_RETRIES: u32 = 10;

/// Helper: wait until TCP port is listening (or timeout)
fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Helper: wait until port is free (or timeout)
fn wait_for_port_free(port: u16, timeout_ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Helper: find a free port and ensure it's not in use
fn find_free_port() -> Result<u16> {
    for _ in 0..MAX_PORT_RETRIES {
        let port = find_available_port(PORT_RANGE_START, PORT_RANGE_END)?;
        
        // Kill any existing process on this port
        kill_process_on_port(port)?;
        
        // Wait for port to be free
        if !wait_for_port_free(port, 1000) {
            continue;
        }
        
        // Double check port is actually free
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            // Wait a bit to ensure the port isn't in TIME_WAIT state
            std::thread::sleep(Duration::from_millis(100));
            
            // Check again to be really sure
            if TcpStream::connect(("127.0.0.1", port)).is_err() {
                return Ok(port);
            }
        }
        
        std::thread::sleep(Duration::from_millis(100));
    }
    
    Err(anyhow::anyhow!("Could not find a free port after {} attempts", MAX_PORT_RETRIES))
}

/// Helper: kill any existing process using the port
fn kill_process_on_port(port: u16) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // First try lsof
        let output = Command::new("lsof")
            .arg("-i")
            .arg(format!(":{}", port))
            .arg("-t")
            .output()?;

        if !output.stdout.is_empty() {
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            let pids: Vec<&str> = stdout_str
                .split('\n')
                .filter(|s| !s.is_empty())
                .collect();
            
            for pid in pids {
                Command::new("kill")
                    .arg("-9")
                    .arg(pid)
                    .output()?;
            }
            
            // Wait for the processes to die
            std::thread::sleep(Duration::from_millis(500));
        }

        // Then try netstat
        let output = Command::new("netstat")
            .arg("-anv")
            .arg("-p")
            .arg("tcp")
            .output()?;

        if !output.stdout.is_empty() {
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            for line in stdout_str.lines() {
                if line.contains(&format!("*.{}", port)) || line.contains(&format!("127.0.0.1.{}", port)) {
                    if let Some(pid) = line.split_whitespace().nth(8) {
                        Command::new("kill")
                            .arg("-9")
                            .arg(pid)
                            .output()?;
                    }
                }
            }
            
            // Wait for the processes to die
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    Ok(())
}

/// Helper: read server output in a separate thread
fn read_server_output(stdout: std::process::ChildStdout, stderr: std::process::ChildStderr) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let tx_clone = tx.clone();

    // Read stdout
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("Server stdout: {}", line);
                if let Err(e) = tx.send(line) {
                    println!("Failed to send stdout line: {}", e);
                    break;
                }
            }
        }
    });

    // Read stderr
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("Server stderr: {}", line);
                if let Err(e) = tx_clone.send(line) {
                    println!("Failed to send stderr line: {}", e);
                    break;
                }
            }
        }
    });

    rx
}

/// Helper: ensure server is running and listening
fn ensure_server_running(child: &mut Child, port: u16) -> Result<()> {
    let start = Instant::now();
    let timeout = Duration::from_millis(SERVER_START_TIMEOUT);

    // Read server output to check for startup messages
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let rx = read_server_output(stdout, stderr);

    // Wait for server to be ready
    while start.elapsed() < timeout {
        // Check if server is listening
        if wait_for_port(port, 100) {
            // Read any remaining output
            while let Ok(line) = rx.try_recv() {
                println!("Server output: {}", line);
                if line.contains("error:") || line.contains("Error:") {
                    return Err(anyhow::anyhow!("Server error: {}", line));
                }
            }
            return Ok(());
        }

        // Check if server process is still alive
        match child.try_wait() {
            Ok(Some(status)) => {
                // Read any remaining output
                while let Ok(line) = rx.try_recv() {
                    println!("Server output: {}", line);
                }
                return Err(anyhow::anyhow!(
                    "Server process exited with status {} before starting",
                    status
                ));
            }
            Ok(None) => {
                // Process is still running, check output
                while let Ok(line) = rx.try_recv() {
                    println!("Server output: {}", line);
                    if line.contains("error:") || line.contains("Error:") {
                        return Err(anyhow::anyhow!("Server error: {}", line));
                    }
                }
                // Continue waiting
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Error checking server process status: {}",
                    e
                ));
            }
        }
    }

    // Timeout exceeded
    Err(anyhow::anyhow!(
        "Server did not start listening on port {} within timeout of {} ms",
        port,
        SERVER_START_TIMEOUT
    ))
}

/// Helper: start server with retries
fn start_server_with_retries(verbose: u8, max_retries: u32) -> Result<(Child, u16)> {
    for attempt in 1..=max_retries {
        let port = find_free_port()?;
        
        // Kill any existing process on this port
        kill_process_on_port(port)?;

        // Wait for port to be free
        if !wait_for_port_free(port, 1000) {
            println!("Port {} still in use after cleanup, retrying...", port);
            continue;
        }

        let mut cmd = Command::new(cargo_bin("dedups"));
        cmd.arg("--server-mode")
            .arg("--port").arg(port.to_string())
            .arg("--use-protobuf")
            .arg("--use-compression")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // propagate verbosity for manual debugging if desired
        for _ in 0..verbose {
            cmd.arg("-v");
        }

        // Set RUST_LOG for better debugging
        cmd.env("RUST_LOG", "debug");

        println!("Starting server with command: {:?}", cmd);
        match cmd.spawn() {
            Ok(mut child) => {
                // Wait for server to start
                match ensure_server_running(&mut child, port) {
                    Ok(_) => {
                        println!("Server started successfully on port {}", port);
                        return Ok((child, port));
                    }
                    Err(e) => {
                        println!("Failed to start server on port {}: {}", port, e);
                        let _ = stop_server(child);
                        if attempt == max_retries {
                            return Err(e);
                        }
                    }
                }
            }
            Err(e) => {
                println!("Failed to spawn server on port {}: {}", port, e);
                if attempt == max_retries {
                    return Err(anyhow::anyhow!("Failed to spawn server: {}", e));
                }
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    Err(anyhow::anyhow!("Failed to start server after {} attempts", max_retries))
}

/// Helper: stop server with timeout and cleanup
fn stop_server(mut child: Child) -> Result<()> {
    // Try graceful shutdown first
    if let Err(e) = child.kill() {
        println!("Warning: Failed to kill server process: {}", e);
    }

    // Wait with timeout
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(SERVER_STOP_TIMEOUT) {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    println!("Warning: Server exited with non-zero status: {}", status);
                }
                return Ok(());
            },
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(anyhow::anyhow!("Error waiting for server to stop: {}", e)),
        }
    }

    // Force kill if timeout exceeded
    println!("Warning: Server did not stop gracefully, forcing termination");
    child.kill()?;
    child.wait()?;
    
    Ok(())
}

/// Spawns a local dedups server in --server-mode on a free port and returns the child and port.
fn spawn_local_server(verbose: u8) -> Result<(Child, u16)> {
    start_server_with_retries(verbose, 3)
}

/// Helper: run test with timeout
fn run_test_with_timeout<F, T>(timeout: Duration, test_fn: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        match test_fn() {
            Ok(result) => {
                let _ = tx.send(Ok(result));
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        }
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => {
            // Wait for thread to finish
            let _ = handle.join();
            result
        }
        Err(_) => {
            // Test timed out
            Err(anyhow::anyhow!("Test timed out after {:?}", timeout))
        }
    }
}

/// Helper: perform handshake with server
fn perform_handshake(client: &mut DedupClient) -> Result<()> {
    println!("Performing handshake");
    let resp = client.execute_command(
        "internal_handshake".to_string(),
        Vec::new(),
        HashMap::new(),
    )?;
    assert!(resp.contains("handshake_ack"), "handshake response missing ack");
    println!("Handshake successful");
    Ok(())
}

/// Helper: verify server is running
fn verify_server_running(server_child: &mut Child) -> Result<()> {
    match server_child.try_wait() {
        Ok(None) => {
            println!("Server is still running (expected)");
            Ok(())
        }
        Ok(Some(status)) => {
            Err(anyhow::anyhow!(
                "Server process exited unexpectedly with status: {}",
                status
            ))
        }
        Err(e) => {
            Err(anyhow::anyhow!(
                "Error checking server process status: {}",
                e
            ))
        }
    }
}

#[test]
#[ignore]
fn tunnel_api_handshake_works() -> Result<()> {
    // opt-in via env so we don't run by default in CI without ssh feature build
    if std::env::var("DEDUPS_SSH_TEST").is_err() {
        eprintln!("DEDUPS_SSH_TEST not set – skipping tunnel_api_handshake_works");
        return Ok(());
    }

    run_test_with_timeout(Duration::from_secs(30), || {
        // Spawn server
        let (mut server_child, port) = spawn_local_server(3)?;

        // Wait until we can connect
        println!("Waiting for server to start on port {}", port);
        assert!(wait_for_port(port, SERVER_START_TIMEOUT), "server did not start listening in time");
        println!("Server is listening");

        // Connect using DedupClient
        let mut client = DedupClient::with_options(
            "127.0.0.1".into(),
            port,
            dedups::options::DedupOptions {
                use_ssh_tunnel: false,  // Don't use SSH tunnel for local test
                tunnel_api_mode: true,
                port,
                use_protobuf: true,
                use_compression: true,
                compression_level: 3,
                keep_alive: true,  // Keep-alive is enabled by default
                ..Default::default()
            }
        );
        
        // Verify initial state
        assert_eq!(client.connection_state(), &ConnectionState::Disconnected);
        assert!(client.last_error().is_none());

        // Connect and verify state
        println!("Attempting to connect to server");
        client.connect()?;
        println!("Connected successfully");
        assert_eq!(client.connection_state(), &ConnectionState::Connected);

        // Perform handshake
        perform_handshake(&mut client)?;

        // Verify server process is still running
        verify_server_running(&mut server_child)?;

        // Disconnect and verify state
        println!("Disconnecting client");
        client.disconnect()?;
        assert_eq!(client.connection_state(), &ConnectionState::Disconnected);

        // Verify server is still running after client disconnect
        verify_server_running(&mut server_child)?;

        // Shut server down
        println!("Shutting down server");
        stop_server(server_child)?;

        Ok(())
    })
}

#[test]
#[ignore]
fn test_keep_alive_functionality() -> Result<()> {
    // opt-in via env so we don't run by default in CI without ssh feature build
    if std::env::var("DEDUPS_SSH_TEST").is_err() {
        eprintln!("DEDUPS_SSH_TEST not set – skipping test_keep_alive_functionality");
        return Ok(());
    }

    run_test_with_timeout(Duration::from_secs(60), || {
        // Spawn server with verbosity
        let (mut server_child, port) = spawn_local_server(3)?;

        // Wait until we can connect
        println!("Waiting for server to start on port {}", port);
        assert!(wait_for_port(port, SERVER_START_TIMEOUT), "server did not start listening in time");
        println!("Server is listening");

        // Connect using DedupClient with keep-alive enabled
        let mut client = DedupClient::with_options(
            "127.0.0.1".into(),
            port,
            dedups::options::DedupOptions {
                use_ssh_tunnel: false,  // Don't use SSH tunnel for local test
                tunnel_api_mode: true,
                port,
                use_protobuf: true,
                use_compression: true,
                compression_level: 3,
                keep_alive: true,  // Enable keep-alive
                ..Default::default()
            }
        );

        // Connect
        println!("Attempting to connect to server");
        client.connect()?;
        println!("Connected successfully");
        assert_eq!(client.connection_state(), &ConnectionState::Connected);

        // Perform initial handshake
        perform_handshake(&mut client)?;

        // Wait for a while to ensure keep-alive messages are exchanged
        println!("Waiting to verify keep-alive...");
        for i in 1..=3 {
            println!("Keep-alive check {}/3", i);
            std::thread::sleep(std::time::Duration::from_secs(10));
            
            // Send a test command to verify connection is still alive
            let response = client.execute_command(
                "dedups".to_string(),
                vec!["--help".to_string()],
                HashMap::new(),
            )?;
            assert!(response.contains("Usage:"), "Response should contain help text after keep-alive period");
            
            // Verify client is still connected
            assert_eq!(client.connection_state(), &ConnectionState::Connected, 
                "Client should still be connected after keep-alive period");
        }

        // Verify server process is still running
        verify_server_running(&mut server_child)?;

        // Disconnect and verify state
        println!("Disconnecting client");
        client.disconnect()?;
        assert_eq!(client.connection_state(), &ConnectionState::Disconnected);

        // Verify server is still running after client disconnect
        verify_server_running(&mut server_child)?;

        // Shut server down
        println!("Shutting down server");
        stop_server(server_child)?;

        Ok(())
    })
}

#[test]
#[ignore]
fn test_tunnel_api_communication() -> Result<()> {
    // opt-in via env so we don't run by default in CI without ssh feature build
    if std::env::var("DEDUPS_SSH_TEST").is_err() {
        eprintln!("DEDUPS_SSH_TEST not set – skipping test_tunnel_api_communication");
        return Ok(());
    }

    run_test_with_timeout(Duration::from_secs(30), || {
        // Spawn server with verbosity
        let (mut server_child, port) = spawn_local_server(3)?;

        // Wait until we can connect
        println!("Waiting for server to start on port {}", port);
        assert!(wait_for_port(port, SERVER_START_TIMEOUT), "server did not start listening in time");
        println!("Server is listening");

        // Connect using DedupClient with protobuf enabled
        let mut client = DedupClient::with_options(
            "127.0.0.1".into(),
            port,
            dedups::options::DedupOptions {
                use_ssh_tunnel: false,  // Don't use SSH tunnel for local test
                tunnel_api_mode: true,
                port,
                use_protobuf: true,
                use_compression: true,
                compression_level: 3,
                keep_alive: true,  // Add keep-alive option
                ..Default::default()
            }
        );

        // Verify initial state
        assert_eq!(client.connection_state(), &ConnectionState::Disconnected);

        // Connect
        println!("Attempting to connect to server");
        client.connect()?;
        println!("Connected successfully");
        assert_eq!(client.connection_state(), &ConnectionState::Connected);

        // Perform initial handshake
        perform_handshake(&mut client)?;

        // Send a test command that the server will understand
        println!("Sending test command");
        let response = client.execute_command(
            "dedups".to_string(),
            vec!["--help".to_string()],  // A safe command that will always work
            HashMap::new(),
        )?;
        println!("Got response");

        // Verify response format (should be protobuf-decoded)
        assert!(!response.contains('{'), "Response should not be JSON when using protobuf");
        assert!(response.contains("Usage:"), "Response should contain help text");

        // Test error handling by sending an invalid command
        println!("Testing error handling with invalid command");
        let err_result = client.execute_command(
            "invalid_command".to_string(),
            vec![],
            HashMap::new(),
        );
        assert!(err_result.is_err(), "Invalid command should return error");
        if let Err(e) = err_result {
            assert!(e.to_string().contains("command"), "Error should mention command");
        }

        // Verify server process is still running
        verify_server_running(&mut server_child)?;

        // Disconnect and verify state
        println!("Disconnecting client");
        client.disconnect()?;
        assert_eq!(client.connection_state(), &ConnectionState::Disconnected);

        // Verify server is still running after client disconnect
        verify_server_running(&mut server_child)?;

        // Shut server down
        println!("Shutting down server");
        stop_server(server_child)?;

        Ok(())
    })
} 