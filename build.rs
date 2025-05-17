use std::io::Result;

fn main() -> Result<()> {
    #[cfg(feature = "proto")]
    compile_protos()?;

    // Always compile successfully even if proto feature is not enabled
    Ok(())
}

#[cfg(feature = "proto")]
fn compile_protos() -> Result<()> {
    use std::fs;
    use std::path::Path;

    // Create proto directory if it doesn't exist
    let proto_dir = Path::new("proto");
    if !proto_dir.exists() {
        fs::create_dir_all(proto_dir)?;
    }

    // Ensure our basic proto file exists
    let proto_file = proto_dir.join("dedups.proto");
    if !proto_file.exists() {
        // Create a basic proto file if it doesn't exist yet
        fs::write(
            &proto_file,
            r#"syntax = "proto3";
package dedups;

// Message types
enum MessageType {
    COMMAND = 0;
    PROGRESS = 1;
    RESULT = 2;
    ERROR = 3;
}

// Core message structure
message DedupMessage {
    MessageType message_type = 1;
    bytes payload = 2;  // Can be JSON or another serialized proto message
}

// Command message from client to server
message CommandMessage {
    string command = 1;
    repeated string args = 2;
    map<string, string> options = 3;
}

// Progress update from server to client
message ProgressMessage {
    uint32 stage = 1;
    string stage_name = 2;
    uint32 files_processed = 3;
    uint32 total_files = 4;
    float percent_complete = 5;
    optional string current_file = 6;
    uint64 bytes_processed = 7;
    uint64 total_bytes = 8;
    double elapsed_seconds = 9;
    optional double estimated_seconds_left = 10;
    string status_message = 11;
}

// Result message from server to client
message ResultMessage {
    uint32 duplicate_count = 1;
    uint32 total_files = 2;
    uint64 total_bytes = 3;
    uint64 duplicate_bytes = 4;
    double elapsed_seconds = 5;
}

// Error message
message ErrorMessage {
    string message = 1;
    int32 code = 2;
}

// Basic options structure that can be shared between CLI, config, and protocol
message DedupOptions {
    // Basic options
    repeated string directories = 1;
    optional string target = 2;
    bool deduplicate = 3;
    bool delete = 4;
    optional string move_to = 5;
    bool json = 6;
    string algorithm = 7;
    optional uint32 parallel = 8;
    string mode = 9;
    bool interactive = 10;
    uint32 verbose = 11;
    repeated string include = 12;
    repeated string exclude = 13;
    optional string filter_from = 14;
    bool progress = 15;
    string sort_by = 16;
    string sort_order = 17;
    bool raw_sizes = 18;
    bool dry_run = 19;
    optional string cache_location = 20;
    bool fast_mode = 21;
    
    // Media options
    bool media_mode = 22;
    string media_resolution = 23;
    repeated string media_formats = 24;
    uint32 media_similarity = 25;
    
    // SSH options
    bool allow_remote_install = 26;
    repeated string ssh_options = 27;
    repeated string rsync_options = 28;
    bool use_remote_dedups = 29;
    bool use_sudo = 30;
    bool use_ssh_tunnel = 31;
    bool server_mode = 32;
    uint32 port = 33;
    
    // Protocol options
    bool use_protobuf = 34;
    bool use_compression = 35;
    uint32 compression_level = 36;
}
"#,
        )?;
    }

    // Compile the proto file
    prost_build::compile_protos(&[proto_file], &[proto_dir])?;

    println!("cargo:rerun-if-changed=proto/dedups.proto");

    Ok(())
}
