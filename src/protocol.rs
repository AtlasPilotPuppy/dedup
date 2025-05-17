// Protocol definitions for client/server communication
// Only compiled when SSH feature is enabled
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Result};
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssh")]
use std::collections::HashMap;
#[cfg(feature = "ssh")]
use std::io::{BufRead, BufReader, Write};
#[cfg(feature = "ssh")]
use std::net::TcpStream;

// Message types for client/server communication
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Command,  // Client to server command
    Progress, // Server to client progress update
    Result,   // Server to client final result
    Error,    // Error message
}

// Core message structure
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DedupMessage {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    pub payload: String, // JSON for specific message content
}

// Command message from client to server
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CommandMessage {
    pub command: String,
    pub args: Vec<String>,
    pub options: HashMap<String, String>,
}

// Progress update from server to client
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProgressMessage {
    pub stage: u8,
    pub stage_name: String,
    pub files_processed: usize,
    pub total_files: usize,
    pub percent_complete: f32,
    pub current_file: Option<String>,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub elapsed_seconds: f64,
    pub estimated_seconds_left: Option<f64>,
    pub status_message: String,
}

// Result message from server to client
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResultMessage {
    pub duplicate_count: usize,
    pub total_files: usize,
    pub total_bytes: u64,
    pub duplicate_bytes: u64,
    pub elapsed_seconds: f64,
}

// Error message
#[cfg(feature = "ssh")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorMessage {
    pub message: String,
    pub code: i32,
}

// Protocol handler trait
#[cfg(feature = "ssh")]
pub trait ProtocolHandler {
    fn handle_message(&mut self, message: &DedupMessage) -> Result<()>;
    fn send_message(&mut self, message: DedupMessage) -> Result<()>;
    fn receive_message(&mut self) -> Result<Option<DedupMessage>>;
}

// Basic protocol handler implementation for TCP streams
#[cfg(feature = "ssh")]
pub struct TcpProtocolHandler {
    pub stream: TcpStream,
    reader: BufReader<TcpStream>,
}

#[cfg(feature = "ssh")]
impl TcpProtocolHandler {
    pub fn new(stream: TcpStream) -> Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }
}

#[cfg(feature = "ssh")]
impl ProtocolHandler for TcpProtocolHandler {
    fn send_message(&mut self, message: DedupMessage) -> Result<()> {
        let json = serde_json::to_string(&message)?;
        log::debug!("Sending message: {}", json);
        self.stream.write_all(json.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;
        Ok(())
    }

    fn receive_message(&mut self) -> Result<Option<DedupMessage>> {
        let mut line = String::new();
        let bytes_read = self.reader.read_line(&mut line)?;

        if bytes_read == 0 {
            return Ok(None); // EOF
        }

        log::debug!("Received message: {}", line);
        let message: DedupMessage = serde_json::from_str(line.trim())?;
        Ok(Some(message))
    }

    fn handle_message(&mut self, message: &DedupMessage) -> Result<()> {
        match message.message_type {
            MessageType::Command => {
                // Default implementation just logs
                log::debug!("Received command: {}", message.payload);
            }
            MessageType::Progress => {
                log::debug!("Received progress update");
            }
            MessageType::Result => {
                log::debug!("Received final result");
            }
            MessageType::Error => {
                log::warn!("Received error: {}", message.payload);
            }
        }
        Ok(())
    }
}

// Utility function to find an available port
#[cfg(feature = "ssh")]
pub fn find_available_port(start_range: u16, end_range: u16) -> Result<u16> {
    for port in start_range..=end_range {
        if let Ok(_) = std::net::TcpListener::bind(format!("127.0.0.1:{}", port)) {
            return Ok(port);
        }
    }
    Err(anyhow!(
        "No available ports found in range {}-{}",
        start_range,
        end_range
    ))
}
