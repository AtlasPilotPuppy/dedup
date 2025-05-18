#[cfg(feature = "ssh")]
mod ssh_tests {
    use crate::ssh_utils::RemoteLocation;
    use std::path::PathBuf;

    #[test]
    fn test_ssh_path_parser() {
        // Basic host and path
        let location = RemoteLocation::parse("ssh:localhost:/tmp").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert!(location.username.is_none());
        assert!(location.port.is_none());
        assert!(location.ssh_options.is_empty());
        assert!(location.rsync_options.is_empty());

        // With username
        let location = RemoteLocation::parse("ssh:username@localhost:/tmp").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert_eq!(location.username, Some("username".to_string()));
        assert!(location.port.is_none());

        // With port
        let location = RemoteLocation::parse("ssh:localhost:2222:/tmp").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert_eq!(location.port, Some(2222));

        // With username and port
        let location = RemoteLocation::parse("ssh:username@localhost:2222:/tmp").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert_eq!(location.username, Some("username".to_string()));
        assert_eq!(location.port, Some(2222));

        // With SSH options
        let location =
            RemoteLocation::parse("ssh:localhost:/tmp:-v,-o,StrictHostKeyChecking=no:").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert_eq!(
            location.ssh_options,
            vec!["-v", "-o", "StrictHostKeyChecking=no"]
        );
        assert!(location.rsync_options.is_empty());

        // With Rsync options
        let location = RemoteLocation::parse("ssh:localhost:/tmp::-z,--info=progress2").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert!(location.ssh_options.is_empty());
        assert_eq!(location.rsync_options, vec!["-z", "--info=progress2"]);

        // With both options
        let location = RemoteLocation::parse("ssh:localhost:/tmp:-v:--progress").unwrap();
        assert_eq!(location.host, "localhost");
        assert_eq!(location.path, PathBuf::from("/tmp"));
        assert_eq!(location.ssh_options, vec!["-v"]);
        assert_eq!(location.rsync_options, vec!["--progress"]);
    }

    #[test]
    fn test_ssh_command_generation() {
        // Basic
        let location = RemoteLocation::parse("ssh:localhost:/tmp").unwrap();
        let cmd = location.ssh_command();
        assert_eq!(cmd, vec!["ssh", "localhost"]);

        // With port
        let location = RemoteLocation::parse("ssh:localhost:2222:/tmp").unwrap();
        let cmd = location.ssh_command();
        assert_eq!(cmd, vec!["ssh", "-p", "2222", "localhost"]);

        // With username
        let location = RemoteLocation::parse("ssh:username@localhost:/tmp").unwrap();
        let cmd = location.ssh_command();
        assert_eq!(cmd, vec!["ssh", "username@localhost"]);

        // With options
        let location =
            RemoteLocation::parse("ssh:localhost:/tmp:-v,-o,StrictHostKeyChecking=no:").unwrap();
        let cmd = location.ssh_command();
        assert_eq!(
            cmd,
            vec!["ssh", "-v", "-o", "StrictHostKeyChecking=no", "localhost"]
        );
    }

    #[test]
    fn test_rsync_command_generation() {
        // Basic source to remote
        let location = RemoteLocation::parse("ssh:localhost:/remote/path").unwrap();
        let source = PathBuf::from("/local/path");
        let dest = PathBuf::from("/remote/path");
        let cmd = location.rsync_command(&source, &dest, false);
        assert_eq!(
            cmd,
            vec!["rsync", "-avz", "/local/path", "localhost:/remote/path"]
        );

        // Basic remote to source
        let cmd = location.rsync_command(&dest, &source, true);
        assert_eq!(
            cmd,
            vec!["rsync", "-avz", "localhost:/remote/path", "/local/path"]
        );

        // With username
        let location = RemoteLocation::parse("ssh:username@localhost:/remote/path").unwrap();
        let cmd = location.rsync_command(&source, &dest, false);
        assert_eq!(
            cmd,
            vec![
                "rsync",
                "-avz",
                "/local/path",
                "username@localhost:/remote/path"
            ]
        );

        // With port
        let location = RemoteLocation::parse("ssh:localhost:2222:/remote/path").unwrap();
        let cmd = location.rsync_command(&source, &dest, false);
        assert_eq!(
            cmd,
            vec![
                "rsync",
                "-avz",
                "-e",
                "ssh -p 2222",
                "/local/path",
                "localhost:/remote/path"
            ]
        );

        // With SSH options
        let location = RemoteLocation::parse("ssh:localhost:/remote/path:-v:").unwrap();
        let cmd = location.rsync_command(&source, &dest, false);
        assert_eq!(
            cmd,
            vec![
                "rsync",
                "-avz",
                "-e",
                "ssh -v",
                "/local/path",
                "localhost:/remote/path"
            ]
        );

        // With Rsync options
        let location = RemoteLocation::parse("ssh:localhost:/remote/path::--progress").unwrap();
        let cmd = location.rsync_command(&source, &dest, false);
        assert_eq!(
            cmd,
            vec![
                "rsync",
                "-avz",
                "--progress",
                "/local/path",
                "localhost:/remote/path"
            ]
        );
    }

    #[test]
    #[ignore] // This test requires a configured SSH host named 'local'
    fn test_local_ssh_integration() {
        // This test requires a local SSH config with a host named 'local'
        // The host should be configured to allow SSH key authentication

        let location = RemoteLocation::parse("ssh:local:/tmp").unwrap();

        // Test basic command execution
        let output = location.run_command("echo 'Test SSH connection'").unwrap();
        assert_eq!(output.trim(), "Test SSH connection");

        // Test file listing
        let output = location.run_command("ls -la /tmp | head -n 2").unwrap();
        assert!(!output.is_empty());

        // Check if dedups is installed
        let dedups_check = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(location.check_dedups_installed());

        println!("Dedups installed on local test host: {:?}", dedups_check);
    }
}
