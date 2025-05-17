// Protocol definitions for client/server communication
// Only compiled when SSH feature is enabled
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Result};
#[cfg(feature = "ssh")]
use bytes::Bytes;
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssh")]
use std::collections::HashMap;
#[cfg(feature = "ssh")]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(feature = "ssh")]
use std::net::TcpStream;

#[cfg(all(feature = "ssh", feature = "proto"))]
use crate::options::proto;

#[cfg(all(feature = "ssh", feature = "proto"))]
use prost::Message;

#[cfg(all(feature = "ssh", feature = "proto"))]
use zstd::{decode_all, encode_all};

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

// Core message structure for JSON protocol
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
pub trait ProtocolHandler: Send {
    fn handle_message(&mut self, message: &DedupMessage) -> Result<()>;
    fn send_message(&mut self, message: DedupMessage) -> Result<()>;
    fn receive_message(&mut self) -> Result<Option<DedupMessage>>;

    // Clone method to create a box with a clone of self
    fn box_clone(&self) -> Box<dyn ProtocolHandler>;
}

// Basic protocol handler implementation for TCP streams using JSON
#[cfg(feature = "ssh")]
pub struct JsonProtocolHandler {
    pub stream: TcpStream,
    reader: BufReader<TcpStream>,
}

#[cfg(feature = "ssh")]
impl JsonProtocolHandler {
    pub fn new(stream: TcpStream) -> Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }
}

#[cfg(feature = "ssh")]
impl ProtocolHandler for JsonProtocolHandler {
    fn send_message(&mut self, message: DedupMessage) -> Result<()> {
        let json = serde_json::to_string(&message)?;
        log::debug!("Sending JSON message: {}", json);
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

        log::debug!("Received JSON message: {}", line);
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

    fn box_clone(&self) -> Box<dyn ProtocolHandler> {
        Box::new(self.clone())
    }
}

#[cfg(feature = "ssh")]
impl Clone for JsonProtocolHandler {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.try_clone().expect("Failed to clone stream"),
            reader: BufReader::new(self.stream.try_clone().expect("Failed to clone stream")),
        }
    }
}

// Protocol handler implementation using Protobuf with optional ZSTD compression
#[cfg(all(feature = "ssh", feature = "proto"))]
pub struct ProtobufProtocolHandler {
    pub stream: TcpStream,
    reader: BufReader<TcpStream>,
    use_compression: bool,
    compression_level: i32,
}

#[cfg(all(feature = "ssh", feature = "proto"))]
impl ProtobufProtocolHandler {
    pub fn new(stream: TcpStream, use_compression: bool, compression_level: i32) -> Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            use_compression,
            compression_level: compression_level.clamp(1, 22), // ZSTD levels are 1-22
        })
    }

    // Compress data using ZSTD
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        if self.use_compression {
            log::debug!(
                "Compressing data with ZSTD level {}",
                self.compression_level
            );
            let compressed = encode_all(data, self.compression_level)?;
            log::debug!(
                "Compressed {} bytes to {} bytes",
                data.len(),
                compressed.len()
            );
            Ok(compressed)
        } else {
            Ok(data.to_vec())
        }
    }

    // Decompress data using ZSTD
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        if self.use_compression {
            log::debug!("Decompressing data with ZSTD");
            let decompressed = decode_all(data)?;
            log::debug!(
                "Decompressed {} bytes to {} bytes",
                data.len(),
                decompressed.len()
            );
            Ok(decompressed)
        } else {
            Ok(data.to_vec())
        }
    }
}

#[cfg(all(feature = "ssh", feature = "proto"))]
impl ProtocolHandler for ProtobufProtocolHandler {
    fn send_message(&mut self, message: DedupMessage) -> Result<()> {
        // Convert JSON-style message to Protobuf message
        let message_type = match message.message_type {
            MessageType::Command => proto::MessageType::Command,
            MessageType::Progress => proto::MessageType::Progress,
            MessageType::Result => proto::MessageType::Result,
            MessageType::Error => proto::MessageType::Error,
        };

        // Create protobuf message
        let proto_msg = proto::DedupMessage {
            message_type: message_type as i32,
            payload: message.payload.into_bytes(),
        };

        // Serialize to bytes
        let mut buf = Vec::new();
        proto_msg.encode(&mut buf)?;

        // Compress if enabled
        let data_to_send = self.compress(&buf)?;

        // Send length as 4-byte prefix, followed by data
        let len = data_to_send.len() as u32;
        let len_bytes = len.to_be_bytes();

        log::debug!(
            "Sending Protobuf message: type={:?}, len={}",
            message_type,
            len
        );

        self.stream.write_all(&len_bytes)?;
        self.stream.write_all(&data_to_send)?;
        self.stream.flush()?;

        Ok(())
    }

    fn receive_message(&mut self) -> Result<Option<DedupMessage>> {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        let bytes_read = self.reader.read(&mut len_buf)?;

        if bytes_read == 0 {
            return Ok(None); // EOF
        }

        if bytes_read < 4 {
            return Err(anyhow!("Incomplete message length prefix"));
        }

        // Convert to u32
        let len = u32::from_be_bytes(len_buf) as usize;
        log::debug!("Received message length: {} bytes", len);

        // Read the message data
        let mut data = vec![0u8; len];
        let mut bytes_remaining = len;
        let mut offset = 0;

        while bytes_remaining > 0 {
            let bytes_read = self.reader.read(&mut data[offset..len])?;
            if bytes_read == 0 {
                return Err(anyhow!("Unexpected EOF while reading message data"));
            }

            offset += bytes_read;
            bytes_remaining -= bytes_read;
        }

        // Decompress if needed
        let decoded_data = self.decompress(&data)?;

        // Decode protobuf message
        let proto_msg = proto::DedupMessage::decode(&*Bytes::from(decoded_data))?;

        // Convert to JSON-style message
        let message_type = match proto::MessageType::try_from(proto_msg.message_type) {
            Ok(proto::MessageType::Command) => MessageType::Command,
            Ok(proto::MessageType::Progress) => MessageType::Progress,
            Ok(proto::MessageType::Result) => MessageType::Result,
            Ok(proto::MessageType::Error) => MessageType::Error,
            Err(e) => return Err(anyhow!("Invalid message type: {}", e)),
        };

        // Convert payload to string
        let payload = String::from_utf8(proto_msg.payload)?;

        log::debug!(
            "Received Protobuf message: type={:?}, payload_len={}",
            message_type,
            payload.len()
        );

        Ok(Some(DedupMessage {
            message_type,
            payload,
        }))
    }

    fn handle_message(&mut self, message: &DedupMessage) -> Result<()> {
        match message.message_type {
            MessageType::Command => {
                log::debug!("Received command via Protobuf: {}", message.payload);
            }
            MessageType::Progress => {
                log::debug!("Received progress update via Protobuf");
            }
            MessageType::Result => {
                log::debug!("Received final result via Protobuf");
            }
            MessageType::Error => {
                log::warn!("Received error via Protobuf: {}", message.payload);
            }
        }
        Ok(())
    }

    fn box_clone(&self) -> Box<dyn ProtocolHandler> {
        Box::new(self.clone())
    }
}

#[cfg(all(feature = "ssh", feature = "proto"))]
impl Clone for ProtobufProtocolHandler {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.try_clone().expect("Failed to clone stream"),
            reader: BufReader::new(self.stream.try_clone().expect("Failed to clone stream")),
            use_compression: self.use_compression,
            compression_level: self.compression_level,
        }
    }
}

// Factory for creating the appropriate protocol handler based on config
#[cfg(feature = "ssh")]
pub fn create_protocol_handler(
    stream: TcpStream,
    use_protobuf: bool,
    use_compression: bool,
    compression_level: u32,
) -> Result<Box<dyn ProtocolHandler>> {
    #[cfg(feature = "proto")]
    {
        if use_protobuf {
            log::info!(
                "Using Protobuf protocol with{} compression",
                if use_compression { "" } else { "out" }
            );
            let handler =
                ProtobufProtocolHandler::new(stream, use_compression, compression_level as i32)?;
            return Ok(Box::new(handler));
        }
    }

    // Default to JSON protocol
    log::info!("Using JSON protocol");
    let handler = JsonProtocolHandler::new(stream)?;
    Ok(Box::new(handler))
}

// For backward compatibility, we'll alias TcpProtocolHandler to JsonProtocolHandler
#[cfg(feature = "ssh")]
pub type TcpProtocolHandler = JsonProtocolHandler;

// Utility function to find an available port
#[cfg(feature = "ssh")]
pub fn find_available_port(start_range: u16, end_range: u16) -> Result<u16> {
    use std::net::TcpListener;

    // Try the default dedups port first (29875) if within range
    let default_port = 29875;
    if default_port >= start_range && default_port <= end_range {
        if TcpListener::bind(("127.0.0.1", default_port)).is_ok() {
            return Ok(default_port);
        }
    }

    // If default port is unavailable, try sequentially in the range
    for port in start_range..=end_range {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(anyhow!("No available ports found in range {}-{}", start_range, end_range))
}
