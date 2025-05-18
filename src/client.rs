#[cfg(feature = "ssh")]
use crate::options::DedupOptions;
#[cfg(feature = "ssh")]
use crate::protocol::{
    create_protocol_handler, CommandMessage, DedupMessage, ErrorMessage, MessageType,
    ProgressMessage, ProtocolHandler,
};
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde_json;
#[cfg(feature = "ssh")]
use std::collections::HashMap;
#[cfg(feature = "ssh")]
use std::io::ErrorKind;
#[cfg(feature = "ssh")]
use std::net::TcpStream;
#[cfg(feature = "ssh")]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
#[cfg(feature = "ssh")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "ssh")]
use std::time::{Duration, Instant};

/// Maximum number of consecutive timeouts before considering connection dead
const MAX_TIMEOUTS: u32 = 50;
/// Default timeout duration for receiving messages
const DEFAULT_RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
/// Keep-alive timeout
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(15);
/// Constants for heartbeat timing
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Client connection state
#[derive(Debug, PartialEq)]
#[cfg(feature = "ssh")]
pub enum ConnectionState {
    /// Client is not connected to any server
    Disconnected,
    /// Client is in the process of connecting
    Connecting,
    /// Client is connected to a server
    Connected,
    /// Connection attempt failed
    Failed,
}

/// Client for communicating with a dedups server
#[cfg(feature = "ssh")]
pub struct DedupClient {
    host: String,
    port: u16,
    protocol: Option<Box<dyn ProtocolHandler>>,
    options: DedupOptions,
    message_receiver: Option<Receiver<DedupMessage>>,
    reader_thread: Option<JoinHandle<()>>,
    verbose: u8,
    state: ConnectionState,
    last_error: Option<String>,
}

#[cfg(feature = "ssh")]
impl DedupClient {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            protocol: None,
            options: DedupOptions::default(),
            message_receiver: None,
            reader_thread: None,
            verbose: 0,
            state: ConnectionState::Disconnected,
            last_error: None,
        }
    }

    pub fn with_options(host: String, port: u16, options: DedupOptions) -> Self {
        let verbose = if let Ok(val) = std::env::var("VERBOSITY") {
            val.parse::<u8>().unwrap_or(0)
        } else {
            0
        };

        #[cfg(feature = "proto")]
        let mut options_with_defaults = options;

        #[cfg(feature = "proto")]
        {
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
            host,
            port,
            protocol: None,
            options: options_with_defaults,
            message_receiver: None,
            reader_thread: None,
            verbose,
            state: ConnectionState::Disconnected,
            last_error: None,
        }
    }

    /// Get the current connection state
    pub fn connection_state(&self) -> &ConnectionState {
        &self.state
    }

    /// Get the last error message if any
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Connect to the dedups server with timeout
    pub fn connect(&mut self) -> Result<()> {
        if self.state == ConnectionState::Connected {
            return Ok(());
        }

        if self.verbose >= 2 {
            log::info!("Connecting to dedups server at {}:{}", self.host, self.port);
        }

        self.state = ConnectionState::Connecting;
        self.last_error = None;

        let stream = TcpStream::connect((self.host.as_str(), self.port))
            .with_context(|| format!("Failed to connect to {}:{}", self.host, self.port))?;

        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                self.options.use_protobuf
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                self.options.use_compression
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = {
            #[cfg(feature = "proto")]
            {
                self.options.compression_level
            }
            #[cfg(not(feature = "proto"))]
            {
                0
            }
        };

        let protocol = create_protocol_handler(
            stream.try_clone()?,
            use_protobuf,
            use_compression,
            compression_level,
        )
        .with_context(|| "Failed to create protocol handler")?;

        let (tx, rx) = mpsc::channel();
        self.message_receiver = Some(rx);

        let mut reader_protocol =
            create_protocol_handler(stream, use_protobuf, use_compression, compression_level)
                .with_context(|| "Failed to create reader protocol handler")?;

        let verbose = self.verbose;
        let keep_alive = self.options.keep_alive;
        let reader_thread = thread::spawn(move || {
            if verbose >= 3 {
                log::debug!("Starting server reader thread");
            }

            let mut last_activity = Instant::now();

            loop {
                match reader_protocol.receive_message() {
                    Ok(Some(msg)) => {
                        last_activity = Instant::now();
                        if verbose >= 3 {
                            log::debug!("Received message type: {:?}", msg.message_type);
                        }

                        // Handle keep-alive pings
                        if keep_alive
                            && msg.message_type == MessageType::Result
                            && msg.payload == "ping"
                        {
                            if verbose >= 3 {
                                log::debug!("Received keep-alive ping");
                            }
                            // Send pong response
                            let pong_msg = DedupMessage {
                                message_type: MessageType::Result,
                                payload: "pong".to_string(),
                            };
                            if let Err(e) = reader_protocol.send_message(pong_msg) {
                                log::error!("Failed to send keep-alive pong: {}", e);
                                break;
                            }
                            continue;
                        }

                        if tx.send(msg).is_err() {
                            log::error!("Failed to send message to channel, receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => {
                        // Check keep-alive timeout
                        if keep_alive && last_activity.elapsed() > KEEP_ALIVE_TIMEOUT {
                            log::error!("Keep-alive timeout exceeded");
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        match e.downcast_ref::<std::io::Error>() {
                            Some(io_err) if io_err.kind() == ErrorKind::TimedOut => {
                                // Read timeout, check keep-alive
                                if keep_alive && last_activity.elapsed() > KEEP_ALIVE_TIMEOUT {
                                    log::error!("Keep-alive timeout exceeded");
                                    break;
                                }
                                continue;
                            }
                            Some(io_err) if io_err.kind() == ErrorKind::WouldBlock => {
                                // Non-blocking read with no data
                                thread::sleep(Duration::from_millis(100));
                                continue;
                            }
                            _ => {
                                log::error!("Error reading from server: {}", e);
                                break;
                            }
                        }
                    }
                }
            }

            if verbose >= 3 {
                log::debug!("Server reader thread exiting");
            }
        });

        self.protocol = Some(protocol);
        self.reader_thread = Some(reader_thread);
        self.state = ConnectionState::Connected;
        self.last_error = None;

        if self.verbose >= 2 {
            log::info!("Successfully connected to dedups server");
        } else {
            log::debug!("Connected to dedups server");
        }

        Ok(())
    }

    /// Send a command to the server with error context
    pub fn send_command(
        &mut self,
        command: String,
        args: Vec<String>,
        options: HashMap<String, String>,
    ) -> Result<()> {
        if self.state != ConnectionState::Connected {
            return Err(anyhow!(
                "Not connected to server. Current state: {:?}",
                self.state
            ));
        }

        let protocol = self
            .protocol
            .as_mut()
            .ok_or_else(|| anyhow!("Protocol handler not initialized"))?;

        let cmd_msg = CommandMessage {
            command: command.clone(),
            args: args.clone(),
            options: options.clone(),
        };

        if self.verbose >= 3 {
            log::debug!("Sending command: {} {}", command, args.join(" "));
        }

        let cmd_json = serde_json::to_string(&cmd_msg)
            .with_context(|| format!("Failed to serialize command: {}", command))?;

        let message = DedupMessage {
            message_type: MessageType::Command,
            payload: cmd_json,
        };

        protocol
            .send_message(message)
            .with_context(|| format!("Failed to send command: {} {}", command, args.join(" ")))?;

        if self.verbose >= 3 {
            log::debug!("Command sent successfully");
        }

        Ok(())
    }

    /// Send a heartbeat to the server
    fn send_heartbeat(&mut self) -> Result<()> {
        if let Some(protocol) = self.protocol.as_mut() {
            if self.verbose >= 3 {
                log::debug!("Sending heartbeat to server");
            }

            let message = DedupMessage {
                message_type: MessageType::Result,
                payload: "heartbeat".to_string(),
            };

            protocol
                .send_message(message)
                .with_context(|| "Failed to send heartbeat")?;

            if self.verbose >= 3 {
                log::debug!("Sent heartbeat");
            }
        }
        Ok(())
    }

    /// Receive the next message from the server with improved timeout handling
    pub fn receive_message(&self) -> Result<Option<DedupMessage>> {
        if self.state != ConnectionState::Connected {
            return Err(anyhow!(
                "Not connected to server. Current state: {:?}",
                self.state
            ));
        }

        let receiver = self
            .message_receiver
            .as_ref()
            .ok_or_else(|| anyhow!("Message receiver not initialized"))?;

        match receiver.recv_timeout(DEFAULT_RECEIVE_TIMEOUT) {
            Ok(msg) => {
                if self.verbose >= 3 {
                    log::debug!("Received message of type {:?}", msg.message_type);
                }

                // Handle keep-alive pong
                if msg.message_type == MessageType::Result && msg.payload == "pong" {
                    if self.verbose >= 3 {
                        log::debug!("Received keep-alive pong");
                    }
                    return Ok(None);
                }

                Ok(Some(msg))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if self.verbose >= 1 {
                    log::error!("Server connection closed unexpectedly");
                }
                Err(anyhow!("Server connection closed"))
            }
        }
    }

    /// Disconnect from the server with cleanup
    pub fn disconnect(&mut self) -> Result<()> {
        if self.state == ConnectionState::Disconnected {
            return Ok(());
        }

        if self.verbose >= 3 {
            log::debug!("Disconnecting from server");
        }

        // Send a clean shutdown message
        if let Some(mut protocol) = self.protocol.take() {
            // Try to send a clean shutdown message and ensure it's delivered
            if self.verbose >= 3 {
                log::debug!("Sending disconnect message to server");
            }
            let disconnect_msg = DedupMessage {
                message_type: MessageType::Result,
                payload: "client_disconnect".to_string(),
            };

            let _ = protocol.send_message(disconnect_msg);

            // Small sleep to ensure the message gets delivered
            std::thread::sleep(Duration::from_millis(50));
        }

        // Join reader thread with timeout
        if let Some(thread) = self.reader_thread.take() {
            if self.verbose >= 3 {
                log::debug!("Waiting for reader thread to terminate");
            }

            let timeout = Duration::from_millis(500);
            let thread_handle = std::thread::spawn(move || {
                let _ = thread.join();
            });

            let start = std::time::Instant::now();
            while start.elapsed() < timeout {
                if thread_handle.is_finished() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            if !thread_handle.is_finished() {
                if self.verbose >= 1 {
                    log::warn!("Reader thread did not terminate within timeout");
                }
            } else if self.verbose >= 3 {
                log::debug!("Reader thread terminated normally");
            }
        }

        // Close message receiver channel
        self.message_receiver.take();
        self.state = ConnectionState::Disconnected;
        self.last_error = None;

        if self.verbose >= 2 {
            log::info!("Disconnected from dedups server");
        } else {
            log::debug!("Disconnected from dedups server");
        }

        Ok(())
    }

    /// Execute a command with improved error handling and timeout management
    pub fn execute_command(
        &mut self,
        command: String,
        args: Vec<String>,
        options: HashMap<String, String>,
    ) -> Result<String> {
        if self.state != ConnectionState::Connected {
            return Err(anyhow!(
                "Not connected to server. Current state: {:?}",
                self.state
            ));
        }

        if self.verbose >= 2 {
            log::info!("Executing command via API: {} {}", command, args.join(" "));
        }

        // For handshake command, add protocol information
        let mut command_options = options.clone();
        if command == "internal_handshake" {
            if self.verbose >= 2 {
                log::info!("Preparing internal handshake with protocol information");
            }

            // Add USE_TUNNEL_API flag for handshake (this is important)
            command_options.insert("USE_TUNNEL_API".to_string(), "true".to_string());

            // Add protocol details
            #[cfg(feature = "proto")]
            {
                command_options.insert(
                    "protocol_type".to_string(),
                    if self.options.use_protobuf {
                        "protobuf"
                    } else {
                        "json"
                    }
                    .to_string(),
                );
                command_options.insert(
                    "compression".to_string(),
                    self.options.use_compression.to_string(),
                );
                command_options.insert(
                    "compression_level".to_string(),
                    self.options.compression_level.to_string(),
                );
            }
            #[cfg(not(feature = "proto"))]
            {
                command_options.insert("protocol_type".to_string(), "json".to_string());
                command_options.insert("compression".to_string(), "false".to_string());
                command_options.insert("compression_level".to_string(), "0".to_string());
            }

            if self.verbose >= 3 {
                log::debug!("Handshake options: {:?}", command_options);
            }
        }

        self.send_command(command.clone(), args.clone(), command_options)
            .with_context(|| format!("Failed to send command: {} {}", command, args.join(" ")))?;

        let mut output = String::new();
        let mut timeout_count = 0;
        let mut last_heartbeat = Instant::now();
        let start_time = Instant::now();

        // Use a fixed timeout of 10 seconds for all commands
        let command_timeout = Duration::from_secs(60);

        // Send initial heartbeat
        if let Err(e) = self.send_heartbeat() {
            log::warn!("Failed to send initial heartbeat: {}", e);
        }

        loop {
            // Send heartbeat if needed
            if last_heartbeat.elapsed() > HEARTBEAT_INTERVAL {
                if let Err(e) = self.send_heartbeat() {
                    log::warn!("Failed to send heartbeat: {}", e);
                }
                last_heartbeat = Instant::now();
            }

            match self.receive_message() {
                Ok(Some(msg)) => {
                    timeout_count = 0;

                    match msg.message_type {
                        MessageType::Result => {
                            if self.verbose >= 3 {
                                log::debug!("Received result from server");
                            }

                            // For handshake, verify protocol match
                            if command == "internal_handshake" {
                                match serde_json::from_str::<serde_json::Value>(&msg.payload) {
                                    Ok(handshake_resp) => {
                                        // Verify status first
                                        let status = handshake_resp
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .ok_or_else(|| {
                                                anyhow!("Handshake response missing status")
                                            })?;

                                        if status != "handshake_ack" {
                                            return Err(anyhow!(
                                                "Unexpected handshake status: {}",
                                                status
                                            ));
                                        }

                                        // Verify protocol details
                                        let protocol =
                                            handshake_resp.get("protocol").ok_or_else(|| {
                                                anyhow!(
                                                    "Handshake response missing protocol details"
                                                )
                                            })?;

                                        let server_proto_type = protocol
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .ok_or_else(|| {
                                                anyhow!(
                                                    "Protocol type missing in handshake response"
                                                )
                                            })?;

                                        #[cfg(feature = "proto")]
                                        {
                                            let client_proto_type = if self.options.use_protobuf {
                                                "protobuf"
                                            } else {
                                                "json"
                                            };

                                            if server_proto_type != client_proto_type {
                                                return Err(anyhow!("Protocol mismatch: client using {}, server using {}", 
                                                    client_proto_type, server_proto_type));
                                            }

                                            // Verify compression settings match
                                            let server_compression = protocol.get("compression")
                                                .and_then(|v| v.as_bool())
                                                .ok_or_else(|| anyhow!("Compression setting missing in handshake response"))?;

                                            if server_compression != self.options.use_compression {
                                                return Err(anyhow!(
                                                    "Compression mismatch: client={}, server={}",
                                                    self.options.use_compression,
                                                    server_compression
                                                ));
                                            }
                                        }
                                        #[cfg(not(feature = "proto"))]
                                        {
                                            if server_proto_type != "json" {
                                                return Err(anyhow!("Protocol mismatch: client using json, server using {}", 
                                                    server_proto_type));
                                            }
                                        }

                                        if self.verbose >= 2 {
                                            log::info!(
                                                "Protocol match confirmed: {}",
                                                server_proto_type
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        return Err(anyhow!(
                                            "Failed to parse handshake response: {} (payload: {})",
                                            e,
                                            msg.payload
                                        ));
                                    }
                                }
                            }

                            // For help command, parse and format the response
                            if command == "dedups" && args.contains(&"--help".to_string()) {
                                if let Ok(help_json) =
                                    serde_json::from_str::<serde_json::Value>(&msg.payload)
                                {
                                    if let Some(message) = help_json.get("message") {
                                        output.push_str(message.as_str().unwrap_or(&msg.payload));
                                    } else {
                                        output.push_str(&msg.payload);
                                    }
                                } else {
                                    output.push_str(&msg.payload);
                                }
                            } else {
                                output.push_str(&msg.payload);
                            }

                            return Ok(output);
                        }
                        MessageType::Error => {
                            let error: ErrorMessage = serde_json::from_str(&msg.payload)
                                .with_context(|| {
                                    format!("Failed to parse error message: {}", msg.payload)
                                })?;

                            if self.verbose >= 1 {
                                log::error!(
                                    "Received error from server: {} (code {})",
                                    error.message,
                                    error.code
                                );
                            }

                            return Err(anyhow!(
                                "Server error (code {}): {}",
                                error.code,
                                error.message
                            ));
                        }
                        MessageType::Progress => {
                            let progress: ProgressMessage = match serde_json::from_str(&msg.payload)
                            {
                                Ok(p) => p,
                                Err(e) => {
                                    log::warn!("Failed to parse progress message: {}", e);
                                    continue;
                                }
                            };

                            if self.verbose >= 2 {
                                log::info!(
                                    "Progress: {}% - {}",
                                    progress.percent_complete,
                                    progress.status_message
                                );
                            }

                            output.push_str(&format!(
                                "Progress: {}% - {}\n",
                                progress.percent_complete, progress.status_message
                            ));

                            timeout_count = 0;
                        }
                        _ => {
                            if self.verbose >= 3 {
                                log::debug!("Received other message type: {:?}", msg.message_type);
                            }
                            output.push_str(&msg.payload);
                            output.push('\n');
                        }
                    }
                }
                Ok(None) => {
                    timeout_count += 1;

                    if timeout_count >= MAX_TIMEOUTS {
                        return Err(anyhow!("Command timed out after {} attempts", MAX_TIMEOUTS));
                    }

                    if start_time.elapsed() >= command_timeout {
                        return Err(anyhow!(
                            "Command execution exceeded timeout of {} seconds",
                            command_timeout.as_secs()
                        ));
                    }

                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    // Check if it's a temporary error
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        match io_err.kind() {
                            ErrorKind::WouldBlock
                            | ErrorKind::TimedOut
                            | ErrorKind::ResourceBusy => {
                                if self.verbose >= 3 {
                                    log::debug!("Temporary error reading from server: {}", io_err);
                                }
                                thread::sleep(Duration::from_millis(10));
                                continue;
                            }
                            ErrorKind::ConnectionReset | ErrorKind::BrokenPipe => {
                                return Err(anyhow!("Server connection closed"));
                            }
                            _ => {
                                return Err(anyhow!("Error reading from server: {}", io_err));
                            }
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }
}
