#!/bin/bash

# Prevent Mac from sleeping while Claude Code is running
# Creates a PID file when work starts

PIDS_DIR="/tmp/claude_working_pids"
LOCK_DIR="/tmp/claude_sleep.lock"

# Acquire lock
while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    sleep 0.1
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

mkdir -p "$PIDS_DIR"

# Register this Claude's parent PID
echo "working" > "$PIDS_DIR/$PPID"

# Count active
count=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')

# If first, enable sleep prevention
if [ "$count" -eq 1 ]; then
    sudo pmset -a disablesleep 1
fi

rmdir "$LOCK_DIR" 2>/dev/null
trap - EXIT
