#!/bin/bash

# Usage: ./mock_ssh.sh remote_command
# This script simulates SSH operations by executing commands against the example_remote directory
# It's designed to be used in place of actual SSH for testing dedups remote functionality

# Get the command to run
CMD="$@"

# If it's a dedups command, transform it to use example_remote as the base directory
if [[ "$CMD" == *"dedups"* ]]; then
  # Replace references to paths with example_remote prefix
  CMD=${CMD//\/example_remote\//\/}
  echo "Executing dedups on remote: $CMD"
  $CMD
else
  # For other commands, just execute them
  eval "$CMD"
fi

exit 0 