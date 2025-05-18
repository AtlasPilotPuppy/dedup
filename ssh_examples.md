## SSH Command Line Examples

Here are some additional examples showing how to use dedups with SSH/remote filesystems:

### Basic Duplicate Detection on Remote Host

Scan a remote directory for duplicates:

```bash
dedups ssh:user@example.com:/home/user/photos --output duplicates.json
```

### Comparing Local and Remote Directories

Find duplicates between local and remote directories:

```bash
dedups /local/photos ssh:user@example.com:/remote/photos --deduplicate
```

### Delete Duplicates on Remote Host

Delete duplicate files on a remote host keeping the newest copies:

```bash
dedups ssh:user@example.com:/remote/photos --delete --mode newest_modified --dry-run
```

(Remove `--dry-run` to actually delete files once you're confident with the selection)

### Move Remote Duplicates to Archive

Move duplicates from a remote directory to a local archive:

```bash
dedups ssh:user@example.com:/remote/photos --move-to /local/archive/duplicates
```

### Cross-Host Deduplication

Find and handle duplicates between two remote hosts:

```bash
dedups ssh:user@server1.com:/data ssh:user@server2.com:/backup --deduplicate
```

### Media File Deduplication

Find similar media files (not just exact duplicates) on a remote host:

```bash
dedups ssh:user@example.com:/photos --media-mode --media-similarity 80
```

### Using SSH Options

Supply custom SSH options:

```bash
dedups ssh:user@example.com:/data:-i,~/.ssh/custom_key,-o,StrictHostKeyChecking=no
```

### Using Rsync Options

Supply custom rsync options for file transfers:

```bash
dedups /local/data ssh:user@example.com:/remote/data::--info=progress2,--no-perms
```

### Complex Example with Multiple Options

```bash
dedups /local/photos ssh:user@example.com:/remote/photos:-i,~/.ssh/custom_key:--info=progress2 \
  --deduplicate --delete --mode newest_modified --media-mode --media-similarity 85 \
  --output duplicates.json --dry-run
``` 