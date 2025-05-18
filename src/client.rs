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
use std::net::{TcpStream, SocketAddr};
#[cfg(feature = "ssh")]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
#[cfg(feature = "ssh")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "ssh")]
use std::time::{Duration, Instant};
#[cfg(feature = "ssh")]
use socket2::{Socket, Domain, Type};
#[cfg(feature = "ssh")]
use std::io::{ErrorKind, Read, Write};

/// Maximum number of consecutive timeouts before considering connection dead
const MAX_TIMEOUTS: u32 = 50;
/// Default timeout duration for receiving messages
const DEFAULT_RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
/// Maximum time to wait for initial connection
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum time to wait for command response
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// Keep-alive interval
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// Keep-alive timeout
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(90);

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
        // Clean up any existing connection
        self.disconnect()?;

        self.state = ConnectionState::Connecting;
        let start_time = Instant::now();

        if self.verbose >= 2 {
            log::info!("Connecting to dedups server at {}:{}", self.host, self.port);
        } else {
            log::debug!("Connecting to dedups server at {}:{}", self.host, self.port);
        }

        let addr = format!("{}:{}", self.host, self.port)
            .parse::<SocketAddr>()
            .with_context(|| format!("Invalid address: {}:{}", self.host, self.port))?;

        // Try to connect with timeout
        let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(stream) => stream,
            Err(e) => {
                self.state = ConnectionState::Failed;
                self.last_error = Some(format!("Connection failed: {}", e));
                return Err(anyhow!("Failed to connect: {}", e));
            }
        };

        // Configure socket options
        stream.set_read_timeout(Some(Duration::from_secs(5)))
            .with_context(|| "Failed to set read timeout")?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))
            .with_context(|| "Failed to set write timeout")?;
        stream.set_nonblocking(true)
            .with_context(|| "Failed to set non-blocking mode")?;

        // Convert to socket2::Socket for platform-specific options
        let socket = Socket::from(stream);
        
        // Set TCP keepalive
        socket.set_keepalive(true)
            .with_context(|| "Failed to set keepalive")?;
        
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
        {
            socket.set_tcp_keepalive(
                &socket2::TcpKeepalive::new()
                    .with_time(Duration::from_secs(60))
                    .with_interval(Duration::from_secs(10))
            ).with_context(|| "Failed to set TCP keepalive options")?;
        }

        // Convert back to TcpStream
        let stream = TcpStream::from(socket);

        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                if self.options.use_protobuf {
                    if self.verbose >= 2 {
                        log::info!("Using Protobuf protocol for API communication");
                    }
                    true
                } else {
                    if self.verbose >= 2 {
                        log::info!("Using JSON protocol for API communication");
                    }
                    false
                }
            }
            #[cfg(not(feature = "proto"))]
            {
                if self.verbose >= 2 {
                    log::info!("Using JSON protocol for API communication (protobuf not available)");
                }
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                if self.options.use_compression && use_protobuf {
                    if self.verbose >= 3 {
                        log::debug!("Compression enabled for Protobuf communication");
                    }
                    true
                } else {
                    false
                }
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = self.options.compression_level;

        let protocol = create_protocol_handler(
            stream.try_clone()?,
            use_protobuf,
            use_compression,
            compression_level,
        ).with_context(|| "Failed to create protocol handler")?;

        let (tx, rx) = mpsc::channel();
        self.message_receiver = Some(rx);

        let mut reader_protocol = create_protocol_handler(
            stream,
            use_protobuf,
            use_compression,
            compression_level,
        ).with_context(|| "Failed to create reader protocol handler")?;

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
                        if keep_alive && msg.message_type == MessageType::Result && msg.payload == "ping" {
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
            return Err(anyhow!("Not connected to server. Current state: {:?}", self.state));
        }

        let protocol = self.protocol.as_mut()
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

        protocol.send_message(message)
            .with_context(|| format!("Failed to send command: {} {}", command, args.join(" ")))?;

        if self.verbose >= 3 {
            log::debug!("Command sent successfully");
        }

        Ok(())
    }

    /// Send a keep-alive ping to the server
    fn send_keep_alive(&mut self) -> Result<()> {
        if let Some(protocol) = self.protocol.as_mut() {
            let cmd_msg = CommandMessage {
                command: "ping".to_string(),
                args: Vec::new(),
                options: HashMap::new(),
            };

            let cmd_json = serde_json::to_string(&cmd_msg)
                .with_context(|| "Failed to serialize ping command")?;
            
            let message = DedupMessage {
                message_type: MessageType::Command,
                payload: cmd_json,
            };

            protocol.send_message(message)
                .with_context(|| "Failed to send keep-alive ping")?;

            if self.verbose >= 3 {
                log::debug!("Sent keep-alive ping");
            }
        }
        Ok(())
    }

    /// Receive the next message from the server with improved timeout handling
    pub fn receive_message(&self) -> Result<Option<DedupMessage>> {
        if self.state != ConnectionState::Connected {
            return Err(anyhow!("Not connected to server. Current state: {:?}", self.state));
        }

        let receiver = self.message_receiver.as_ref()
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
            },
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if self.verbose >= 1 {
                    log::error!("Server connection closed unexpectedly");
                }
                Err(anyhow!("Server connection closed"))
            },
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
        
        // Drop protocol handler to close connection
        if let Some(mut protocol) = self.protocol.take() {
            // Try to send a clean shutdown message
            let _ = protocol.send_message(DedupMessage {
                message_type: MessageType::Result,
                payload: "client_disconnect".to_string(),
            });
        }

        // Join reader thread with timeout
        if let Some(thread) = self.reader_thread.take() {
            match thread.join() {
                Ok(_) => {
                    if self.verbose >= 3 {
                        log::debug!("Reader thread terminated normally");
                    }
                }
                Err(e) => {
                    log::warn!("Reader thread panicked: {:?}", e);
                }
            }
        }

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
            return Err(anyhow!("Not connected to server. Current state: {:?}", self.state));
        }

        if self.verbose >= 2 {
            log::info!("Executing command via API: {} {}", command, args.join(" "));
        }
        
        self.send_command(command.clone(), args.clone(), options.clone())
            .with_context(|| format!("Failed to send command: {} {}", command, args.join(" ")))?;

        let mut output = String::new();
        let mut has_error = false;
        let mut timeout_count = 0;
        let start_time = Instant::now();
        let mut got_result = false;
        let mut last_activity = Instant::now();

        loop {
            // Send keep-alive ping if needed
            if self.options.keep_alive && last_activity.elapsed() > KEEP_ALIVE_INTERVAL {
                if let Err(e) = self.send_keep_alive() {
                    log::warn!("Failed to send keep-alive ping: {}", e);
                }
                last_activity = Instant::now();
            }

            match self.receive_message() {
                Ok(Some(msg)) => {
                    last_activity = Instant::now();
                    timeout_count = 0;

                    match msg.message_type {
                        MessageType::Result => {
                            if self.verbose >= 3 {
                                log::debug!("Received final result from server");
                            }
                            output.push_str(&msg.payload);
                            got_result = true;
                            break;
                        }
                        MessageType::Error => {
                            let error: ErrorMessage = serde_json::from_str(&msg.payload)
                                .with_context(|| format!("Failed to parse error message: {}", msg.payload))?;
                            
                            if self.verbose >= 1 {
                                log::error!("Received error from server: {} (code {})", error.message, error.code);
                            }
                            
                            has_error = true;
                            output = format!("Error (code {}): {}", error.code, error.message);
                            break;
                        }
                        MessageType::Progress => {
                            let progress: ProgressMessage = match serde_json::from_str(&msg.payload) {
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
                    // If we got a result but the connection closed, that's okay
                    if got_result {
                        break;
                    }

                    timeout_count += 1;
                    
                    if timeout_count >= MAX_TIMEOUTS {
                        return Err(anyhow!("Command timed out after {} attempts", MAX_TIMEOUTS));
                    }
                    
                    if start_time.elapsed() >= COMMAND_TIMEOUT {
                        return Err(anyhow!("Command execution exceeded timeout of {} seconds", COMMAND_TIMEOUT.as_secs()));
                    }
                    
                    thread::sleep(DEFAULT_RECEIVE_TIMEOUT);
                }
                Err(e) => {
                    // If we got a result but hit an error reading more, that's okay
                    if got_result {
                        break;
                    }

                    // Check if it's a temporary error
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        match io_err.kind() {
                            ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ResourceBusy => {
                                if self.verbose >= 3 {
                                    log::debug!("Temporary error reading from server: {}", io_err);
                                }
                                thread::sleep(Duration::from_millis(100));
                                continue;
                            }
                            ErrorKind::ConnectionReset | ErrorKind::BrokenPipe => {
                                // Connection was closed by server
                                if got_result {
                                    break;
                                }
                                return Err(anyhow!("Server connection closed"));
                            }
                            _ => {
                                return Err(e.context("Error reading from server"));
                            }
                        }
                    } else {
                        return Err(e.context("Error reading from server"));
                    }
                }
            }
        }

        if self.verbose >= 3 {
            log::debug!("Command execution completed");
        }

        // Clean up connection if server closed it
        if self.state == ConnectionState::Connected {
            match self.receive_message() {
                Ok(None) | Err(_) => {
                    // Server closed connection, clean up
                    self.disconnect()?;
                }
                _ => {}
            }
        }

        if has_error {
            Err(anyhow!(output))
        } else {
            Ok(output)
        }
    }
}
