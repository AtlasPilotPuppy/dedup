#[cfg(feature = "ssh")]
use crate::options::DedupOptions;
#[cfg(feature = "ssh")]
use crate::protocol::{
    create_protocol_handler, CommandMessage, DedupMessage, ErrorMessage, MessageType,
    ProtocolHandler,
};
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde_json;
#[cfg(feature = "ssh")]
use socket2;
#[cfg(feature = "ssh")]
use std::io::{BufRead, BufReader};
#[cfg(feature = "ssh")]
use std::net::{TcpListener, TcpStream};
#[cfg(feature = "ssh")]
use std::process::{Command, Stdio};
#[cfg(feature = "ssh")]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[cfg(feature = "ssh")]
use std::thread;
#[cfg(feature = "ssh")]
use std::time::{Duration, Instant};

/// Server implementation that listens for client commands and executes dedups operations
#[cfg(feature = "ssh")]
pub struct DedupServer {
    port: u16,
    running: Arc<AtomicBool>,
    options: Arc<Mutex<DedupOptions>>,
    verbose: u8,
    start_time: std::time::Instant,
}

#[cfg(feature = "ssh")]
impl DedupServer {
    pub fn new(port: u16) -> Self {
        // Get verbosity from environment if set
        let verbose = if let Ok(val) = std::env::var("RUST_LOG") {
            if val.contains("debug") {
                3
            } else if val.contains("info") {
                2
            } else if val.contains("warn") {
                1
            } else {
                0
            }
        } else {
            0
        };

        // Create default options with Protobuf and compression enabled
        let mut default_options = DedupOptions::default();

        #[cfg(feature = "proto")]
        {
            default_options.use_protobuf = true;
            default_options.use_compression = true;
            default_options.compression_level = 3;
            default_options.keep_alive = true; // Enable keep-alive by default
        }

        Self {
            port,
            running: Arc::new(AtomicBool::new(false)),
            options: Arc::new(Mutex::new(default_options)),
            verbose,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn with_options(port: u16, options: DedupOptions) -> Self {
        // Get verbosity from environment if set
        let verbose = if let Ok(val) = std::env::var("VERBOSITY") {
            val.parse::<u8>().unwrap_or(0)
        } else if let Ok(val) = std::env::var("RUST_LOG") {
            if val.contains("debug") {
                3
            } else if val.contains("info") {
                2
            } else if val.contains("warn") {
                1
            } else {
                0
            }
        } else {
            // Get from options if set
            options.verbose
        };

        // Make a copy with defaults applied for proto features
        #[cfg(feature = "proto")]
        let mut options_with_defaults = options;

        #[cfg(feature = "proto")]
        {
            // Apply defaults for protocol features unless explicitly disabled
            if !options_with_defaults.use_protobuf {
                options_with_defaults.use_protobuf = true;
            }
            if !options_with_defaults.use_compression {
                options_with_defaults.use_compression = true;
            }
            if options_with_defaults.compression_level == 0 {
                options_with_defaults.compression_level = 3;
            }
        }

        #[cfg(not(feature = "proto"))]
        let options_with_defaults = options;

        Self {
            port,
            running: Arc::new(AtomicBool::new(false)),
            options: Arc::new(Mutex::new(options_with_defaults)),
            verbose,
            start_time: std::time::Instant::now(),
        }
    }

    /// Start the server on the given port
    pub fn start(&mut self) -> Result<()> {
        // Try to log server startup with PID for identifying the process
        if self.verbose >= 1 {
            let pid = std::process::id();
            log::info!(
                "Starting dedups server on port {} (PID: {})",
                self.port,
                pid
            );
            if self.verbose >= 2 {
                let options = self.options.lock().unwrap();
                #[cfg(feature = "proto")]
                {
                    log::info!(
                        "Server options: protocol={}, compression={}, keep_alive={}",
                        if options.use_protobuf {
                            "protobuf"
                        } else {
                            "json"
                        },
                        if options.use_compression {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        if options.keep_alive {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                }
                #[cfg(not(feature = "proto"))]
                {
                    log::info!("Server options: protocol=json, compression=disabled");
                }
            }
        }

        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .with_context(|| format!("Failed to bind to port {} for dedups server", self.port))?;

        listener
            .set_nonblocking(true)
            .context("Failed to set listener to non-blocking mode")?;

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let options = self.options.clone();
        let verbose = self.verbose;

        // Set up signal handler for graceful shutdown (on unix platforms)
        #[cfg(unix)]
        {
            let running_clone = running.clone();

            match ctrlc::set_handler(move || {
                log::info!("Received termination signal, shutting down server gracefully");
                running_clone.store(false, Ordering::SeqCst);
            }) {
                Ok(_) => {
                    if verbose >= 2 {
                        log::info!("Signal handler installed for graceful shutdown");
                    }
                }
                Err(e) => {
                    log::warn!("Failed to set signal handler: {}", e);
                }
            }
        }

        if self.verbose >= 1 {
            log::info!("Dedups server started on port {}", self.port);
        }

        while running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    if verbose >= 1 {
                        log::info!("New client connection from: {}", addr);
                    }

                    // Clone the stream for error handling
                    let error_stream = match stream.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            log::error!("Failed to clone stream for error handling: {}", e);
                            continue;
                        }
                    };

                    // Handle client in a new thread
                    let running_clone = running.clone();
                    let options_clone = options.clone();
                    thread::spawn(move || {
                        if let Err(e) =
                            Self::handle_client(stream, running_clone, options_clone, verbose)
                        {
                            log::error!("Error handling client: {}", e);
                            // Try to send error to client if possible
                            if let Ok(mut protocol) =
                                create_protocol_handler(error_stream, false, false, 0)
                            {
                                let _ = Self::send_error(
                                    &mut *protocol,
                                    &format!("Server error: {}", e),
                                    500,
                                );
                            }
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection available, sleep briefly
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    log::error!("Error accepting connection: {}", e);
                    break;
                }
            }
        }

        // Print server shutdown information including uptime
        let uptime = self.start_time.elapsed();
        log::info!(
            "Dedups server stopped after being online for {:.1} seconds",
            uptime.as_secs_f64()
        );

        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        if self.verbose >= 1 {
            log::info!("Server shutdown initiated");
        }
        self.running.store(false, Ordering::SeqCst);
    }

    /// Handle a client connection
    fn handle_client(
        stream: TcpStream,
        running: Arc<AtomicBool>,
        options: Arc<Mutex<DedupOptions>>,
        verbose: u8,
    ) -> Result<()> {
        // Configure socket options for keep-alive
        let socket = socket2::Socket::from(stream.try_clone()?);
        socket
            .set_keepalive(true)
            .with_context(|| "Failed to set keep-alive")?;

        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
        {
            use socket2::TcpKeepalive;
            socket
                .set_tcp_keepalive(
                    &TcpKeepalive::new()
                        .with_time(Duration::from_secs(60))
                        .with_interval(Duration::from_secs(10)),
                )
                .with_context(|| "Failed to set TCP keep-alive options")?;
        }

        // Convert back to TcpStream
        let stream = TcpStream::from(socket);

        // Get protocol options from shared state
        let opts = options
            .lock()
            .map_err(|_| anyhow!("Failed to lock options"))?;

        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                let protobuf_enabled = opts.use_protobuf;
                if verbose >= 2 && protobuf_enabled {
                    log::info!("Client using Protobuf protocol");
                }
                protobuf_enabled
            }
            #[cfg(not(feature = "proto"))]
            {
                if verbose >= 2 {
                    log::info!("Client using JSON protocol (Protobuf not available)");
                }
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                let compression_enabled = opts.use_compression;
                if verbose >= 2 && compression_enabled && use_protobuf {
                    log::info!("Compression enabled for Protobuf communication");
                }
                compression_enabled
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = {
            #[cfg(feature = "proto")]
            {
                if verbose >= 3 && use_compression {
                    log::debug!("Using compression level {}", opts.compression_level);
                }
                opts.compression_level
            }
            #[cfg(not(feature = "proto"))]
            {
                3
            }
        };

        let keep_alive = opts.keep_alive;
        drop(opts); // Release the lock

        let mut protocol =
            match create_protocol_handler(stream, use_protobuf, use_compression, compression_level)
            {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Failed to create protocol handler: {}", e);
                    return Err(e);
                }
            };

        // Set up client session start time for tracking
        let session_start = std::time::Instant::now();
        let mut last_activity = std::time::Instant::now();
        let mut last_ping = std::time::Instant::now();

        if verbose >= 2 {
            log::info!("Client connected and ready to receive commands");
        }

        // Set a flag to detect client disconnection
        let mut client_disconnected = false;

        // Keep-alive settings
        let keep_alive_interval = Duration::from_secs(5); // Reduced from 30s to 5s
        let keep_alive_timeout = Duration::from_secs(15); // Reduced from 90s to 15s

        while running.load(Ordering::SeqCst) && !client_disconnected {
            // Send keep-alive ping if needed
            if keep_alive && last_ping.elapsed() > keep_alive_interval {
                if verbose >= 3 {
                    log::debug!("Sending keep-alive ping to client");
                }
                let ping_msg = DedupMessage {
                    message_type: MessageType::Result,
                    payload: "ping".to_string(),
                };
                if let Err(e) = protocol.send_message(ping_msg) {
                    log::error!("Failed to send keep-alive ping: {}", e);
                    client_disconnected = true;
                    continue;
                }
                last_ping = Instant::now();
            }

            // Check keep-alive timeout
            if keep_alive && last_activity.elapsed() > keep_alive_timeout {
                log::error!(
                    "Keep-alive timeout exceeded ({} seconds)",
                    keep_alive_timeout.as_secs()
                );
                client_disconnected = true;
                continue;
            }

            match protocol.receive_message() {
                Ok(Some(message)) => {
                    last_activity = std::time::Instant::now();
                    if verbose >= 3 {
                        log::debug!("Received message type: {:?}", message.message_type);
                    }

                    match message.message_type {
                        MessageType::Command => {
                            // Parse command to check for handshake
                            if let Ok(cmd_msg) =
                                serde_json::from_str::<CommandMessage>(&message.payload)
                            {
                                if cmd_msg.command == "internal_handshake" && verbose >= 1 {
                                    log::info!("Received handshake command - client identified");
                                }
                            }

                            if let Err(e) = Self::handle_command(&mut *protocol, &message, verbose)
                            {
                                log::error!("Error handling command: {}", e);
                                // Try to send error back to client
                                let _ = Self::send_error(
                                    &mut *protocol,
                                    &format!("Error handling command: {}", e),
                                    500,
                                );
                                client_disconnected = true;
                                continue;
                            }
                        }
                        MessageType::Result => {
                            // Check for client disconnect message
                            if message.payload == "client_disconnect" {
                                if verbose >= 1 {
                                    log::info!("Client sent disconnect message");
                                }
                                client_disconnected = true;
                                continue;
                            } else if message.payload == "pong" {
                                // Handle keep-alive pong response
                                if verbose >= 3 {
                                    log::debug!("Received keep-alive pong from client");
                                }
                                last_activity = Instant::now();
                            } else {
                                log::warn!(
                                    "Received unexpected result message: {}",
                                    message.payload
                                );
                            }
                        }
                        _ => {
                            log::warn!(
                                "Received unexpected message type: {:?}",
                                message.message_type
                            );
                            if let Err(e) = Self::send_error(
                                &mut *protocol,
                                "Unexpected message type, expected command",
                                1,
                            ) {
                                log::error!("Failed to send error response: {}", e);
                                client_disconnected = true;
                                continue;
                            }
                        }
                    }
                }
                Ok(None) => {
                    // Check keep-alive timeout
                    if keep_alive && last_activity.elapsed() > keep_alive_timeout {
                        log::error!("Keep-alive timeout exceeded");
                        client_disconnected = true;
                        continue;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    match e.downcast_ref::<std::io::Error>() {
                        Some(io_err) if io_err.kind() == std::io::ErrorKind::TimedOut => {
                            // Read timeout, check keep-alive
                            if keep_alive && last_activity.elapsed() > keep_alive_timeout {
                                log::error!("Keep-alive timeout exceeded");
                                client_disconnected = true;
                                continue;
                            }
                            continue;
                        }
                        Some(io_err) if io_err.kind() == std::io::ErrorKind::WouldBlock => {
                            // Non-blocking read with no data
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        Some(io_err) if io_err.kind() == std::io::ErrorKind::BrokenPipe => {
                            log::warn!("Client connection closed (broken pipe)");
                            client_disconnected = true;
                            continue;
                        }
                        Some(io_err) if io_err.kind() == std::io::ErrorKind::ConnectionReset => {
                            log::warn!("Client connection reset");
                            client_disconnected = true;
                            continue;
                        }
                        Some(io_err) if io_err.kind() == std::io::ErrorKind::ConnectionAborted => {
                            log::warn!("Client connection aborted");
                            client_disconnected = true;
                            continue;
                        }
                        _ => {
                            log::error!("Error reading from client: {}", e);
                            client_disconnected = true;
                            continue;
                        }
                    }
                }
            }
        }

        if verbose >= 1 {
            let session_duration = session_start.elapsed();
            log::info!(
                "Client session ended after {:.1} seconds",
                session_duration.as_secs_f64()
            );
        }

        Ok(())
    }

    /// Send a result message to the client
    fn send_result(protocol: &mut dyn ProtocolHandler, payload: &str, verbose: u8) -> Result<()> {
        if verbose >= 3 {
            log::debug!("Sending result to client: {}", payload);
        }

        let result_msg = DedupMessage {
            message_type: MessageType::Result,
            payload: payload.to_string(),
        };

        match protocol.send_message(result_msg) {
            Ok(_) => Ok(()),
            Err(e) => {
                log::error!("Failed to send result message to client: {}", e);
                Err(e)
            }
        }
    }

    /// Handle a command message
    fn handle_command(
        protocol: &mut dyn ProtocolHandler,
        message: &DedupMessage,
        verbose: u8,
    ) -> Result<()> {
        let command_msg: CommandMessage = match serde_json::from_str(&message.payload) {
            Ok(cmd) => cmd,
            Err(e) => {
                log::error!("Failed to parse command message: {}", e);
                return Self::send_error(protocol, &format!("Failed to parse command: {}", e), 400);
            }
        };

        if verbose >= 1 {
            log::info!(
                "Executing command: {} with {} args",
                command_msg.command,
                command_msg.args.len()
            );

            if verbose >= 3 {
                log::debug!("Command arguments: {}", command_msg.args.join(" "));
                log::debug!("Command options: {:?}", command_msg.options);
            }
        }

        // Special handling for internal_handshake command - respond immediately with a success message
        if command_msg.command == "internal_handshake" {
            if verbose >= 1 {
                log::info!(
                    "Processing internal_handshake request with options: {:?}",
                    command_msg.options
                );
            }

            // Extract protocol info from command options
            let proto_type = command_msg
                .options
                .get("protocol_type")
                .map(|s| s.as_str())
                .unwrap_or("json");
            let compression = command_msg
                .options
                .get("compression")
                .and_then(|s| s.parse::<bool>().ok())
                .unwrap_or(false);
            let compression_level = command_msg
                .options
                .get("compression_level")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(3);

            if verbose >= 2 {
                log::info!(
                    "Handshake details - protocol: {}, compression: {}, level: {}",
                    proto_type,
                    compression,
                    compression_level
                );
            }

            // Send handshake success response with protocol details
            let result_payload = serde_json::json!({
                "status": "handshake_ack",
                "message": "Server ready and acknowledged handshake.",
                "server_version": env!("CARGO_PKG_VERSION"),
                "keep_alive": true,
                "protocol": {
                    "type": proto_type,
                    "compression": compression,
                    "compression_level": compression_level
                }
            })
            .to_string();

            if verbose >= 3 {
                log::debug!("Sending handshake response: {}", result_payload);
            }

            if let Err(e) = Self::send_result(protocol, &result_payload, verbose) {
                log::error!("Failed to send internal_handshake response: {}", e);
                return Err(anyhow::anyhow!(
                    "Failed to send internal_handshake response: {}",
                    e
                ));
            }

            if verbose >= 1 {
                log::info!("Server communication established via internal_handshake.");
            }

            return Ok(());
        }

        // Handle invalid commands
        if command_msg.command != "dedups" {
            let err_msg = format!(
                "Invalid command: {}. Only 'dedups' command is supported.",
                command_msg.command
            );
            log::warn!("{}", err_msg);
            return Self::send_error(protocol, &err_msg, 2);
        }

        // Special handling for dedups commands
        if command_msg.command == "dedups" {
            // Handle help command
            if command_msg.args.contains(&"--help".to_string()) {
                let help_text = serde_json::json!({
                    "type": "result",
                    "message": "Usage: dedups [OPTIONS] [DIRECTORIES]...\n\nA tool for finding and managing duplicate files\n\nOptions:\n  --help                  Print help information\n  --version               Print version information"
                }).to_string();

                if let Err(e) = Self::send_result(protocol, &help_text, verbose) {
                    log::error!("Failed to send help text: {}", e);
                    return Err(anyhow::anyhow!("Failed to send help text: {}", e));
                }

                return Ok(());
            }

            // Handle version command
            if command_msg.args.contains(&"--version".to_string()) {
                let version_text = serde_json::json!({
                    "type": "result",
                    "message": format!("dedups {}", env!("CARGO_PKG_VERSION"))
                })
                .to_string();

                if let Err(e) = Self::send_result(protocol, &version_text, verbose) {
                    log::error!("Failed to send version text: {}", e);
                    return Err(anyhow::anyhow!("Failed to send version text: {}", e));
                }

                return Ok(());
            }
        }

        // For all other dedups commands, execute as a child process
        let mut cmd = Command::new(&command_msg.command);
        cmd.args(&command_msg.args);

        // Check if we're in tunnel API mode where we need strict JSON output separation
        let tunnel_api_mode = command_msg.options.contains_key("USE_TUNNEL_API");

        if tunnel_api_mode {
            if verbose >= 2 {
                log::info!("Using tunnel API mode with strict JSON separation");
            }

            // For tunnel API mode, we force --json output to ensure proper protocol format
            if !command_msg.args.contains(&"--json".to_string()) {
                if verbose >= 2 {
                    log::info!("Adding --json flag for API communication");
                }
                cmd.arg("--json");
            }

            // In tunnel API mode, redirect all logging to stderr only
            cmd.env("RUST_LOG_TARGET", "stderr");

            // Add API mode flag to signal the child process to use strict mode
            cmd.env("DEDUPS_TUNNEL_API", "1");
        }

        // Set up pipes for stdout and stderr
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Add environment variables from options
        for (key, value) in &command_msg.options {
            if key.starts_with("ENV_") {
                let env_name = key.trim_start_matches("ENV_");
                if verbose >= 3 {
                    log::debug!("Setting environment variable: {}={}", env_name, value);
                }
                cmd.env(env_name, value);
            }
        }

        // Execute the command
        let command_start = std::time::Instant::now();

        match cmd.spawn() {
            Ok(mut child) => {
                if verbose >= 2 {
                    log::info!("Command process started");
                }

                let stdout = child.stdout.take().expect("Failed to open stdout");
                let stderr = child.stderr.take().expect("Failed to open stderr");

                // Process stdout in a separate thread
                let mut protocol_clone = protocol.box_clone();
                let tunnel_api_mode_clone = tunnel_api_mode;
                let thread_verbose = verbose;

                let (tx, rx) = std::sync::mpsc::channel();
                let stdout_thread = thread::spawn(move || {
                    if thread_verbose >= 3 {
                        log::debug!("Started stdout processing thread");
                    }

                    let reader = BufReader::new(stdout);
                    let mut line_count = 0;

                    for line in reader.lines().map_while(Result::ok) {
                        line_count += 1;

                        // Try to parse as JSON
                        if line.starts_with('{') && line.ends_with('}') {
                            if thread_verbose >= 3 {
                                log::debug!("Processing JSON output line {}", line_count);
                            }

                            // Forward as Result message
                            let result_msg = DedupMessage {
                                message_type: MessageType::Result,
                                payload: line,
                            };

                            if let Err(e) = protocol_clone.send_message(result_msg) {
                                log::error!("Error sending output to client: {}", e);
                                break;
                            }
                        } else if tunnel_api_mode_clone {
                            // In tunnel API mode, non-JSON stdout is treated as an error
                            log::warn!(
                                "Unexpected non-JSON output on stdout in tunnel API mode: {}",
                                line
                            );
                        } else {
                            // Regular mode, stdout may contain mixed output
                            if thread_verbose >= 2 {
                                log::debug!("STDOUT: {}", line);
                            }

                            // Send as plain text result
                            let result_msg = DedupMessage {
                                message_type: MessageType::Result,
                                payload: line,
                            };

                            if let Err(e) = protocol_clone.send_message(result_msg) {
                                log::error!("Error sending output to client: {}", e);
                                break;
                            }
                        }
                    }

                    if thread_verbose >= 3 {
                        log::debug!(
                            "Stdout processing thread completed, processed {} lines",
                            line_count
                        );
                    }

                    // Signal completion
                    let _ = tx.send(());
                });

                // Process stderr in a separate thread to avoid blocking
                let stderr_verbose = verbose;
                let _stderr_thread = thread::spawn(move || {
                    if stderr_verbose >= 3 {
                        log::debug!("Started stderr processing thread");
                    }

                    let stderr_reader = BufReader::new(stderr);
                    let mut line_count = 0;

                    for line in stderr_reader.lines().map_while(Result::ok) {
                        line_count += 1;
                        // Stderr is always logged but doesn't affect protocol
                        if stderr_verbose >= 2 {
                            log::debug!("STDERR: {}", line);
                        }
                    }

                    if stderr_verbose >= 3 {
                        log::debug!(
                            "Stderr processing thread completed, processed {} lines",
                            line_count
                        );
                    }
                });

                // Wait for stdout thread to complete with timeout
                let stdout_timeout = Duration::from_secs(5); // Reduced from 30s to 5s
                let stdout_thread_done = match rx.recv_timeout(stdout_timeout) {
                    Ok(_) => {
                        if verbose >= 3 {
                            log::debug!("Stdout thread completed within timeout");
                        }
                        true
                    }
                    Err(_) => {
                        log::warn!("Stdout thread did not complete within timeout");
                        false
                    }
                };

                // Wait for process to exit with timeout
                let process_timeout = Duration::from_secs(5); // Reduced from 30s to 5s
                let start = Instant::now();
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break status,
                        Ok(None) => {
                            if start.elapsed() > process_timeout {
                                // Kill the process if it's taking too long
                                let _ = child.kill();
                                return Self::send_error(protocol, "Command timed out", 500);
                            }
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        Err(e) => {
                            log::error!("Failed to wait for command process: {}", e);
                            return Self::send_error(
                                protocol,
                                &format!("Failed to wait for command: {}", e),
                                500,
                            );
                        }
                    }
                };

                let command_duration = command_start.elapsed();

                if verbose >= 1 {
                    log::info!(
                        "Command exited with status: {} after {:.1} seconds",
                        status,
                        command_duration.as_secs_f64()
                    );
                }

                if !status.success() {
                    let err_msg = format!("Command failed with exit code: {}", status);
                    log::warn!("{}", err_msg);
                    Self::send_error(protocol, &err_msg, 2)?;
                } else if verbose >= 2 {
                    log::info!("Command completed successfully");
                }

                // Clean up threads
                if !stdout_thread_done {
                    // Try to join the thread to avoid leaks
                    let _ = stdout_thread.join();
                }
            }
            Err(e) => {
                let err_msg = format!("Failed to execute command: {}", e);
                log::error!("{}", err_msg);
                Self::send_error(protocol, &err_msg, 3)?;
            }
        }

        Ok(())
    }

    /// Send an error message to the client
    fn send_error(protocol: &mut dyn ProtocolHandler, message: &str, code: i32) -> Result<()> {
        log::warn!("Sending error to client: {} (code {})", message, code);

        let error = ErrorMessage {
            message: message.to_string(),
            code,
        };

        let error_json = match serde_json::to_string(&error) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize error message: {}", e);
                return Err(anyhow!("Failed to serialize error message: {}", e));
            }
        };

        let error_msg = DedupMessage {
            message_type: MessageType::Error,
            payload: error_json,
        };

        match protocol.send_message(error_msg) {
            Ok(_) => Ok(()),
            Err(e) => {
                log::error!("Failed to send error message to client: {}", e);
                Err(e)
            }
        }
    }
}

/// Entry point for running the server
#[cfg(feature = "ssh")]
pub fn run_server(port: u16) -> Result<()> {
    let mut server = DedupServer::new(port);
    server.start()?;
    Ok(())
}

/// Entry point for running the server with custom options
#[cfg(feature = "ssh")]
pub fn run_server_with_options(port: u16, options: DedupOptions) -> Result<()> {
    let mut server = DedupServer::with_options(port, options);
    server.start()?;
    Ok(())
}

/// Check if the server is already running
#[cfg(feature = "ssh")]
pub fn check_server_running(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}
