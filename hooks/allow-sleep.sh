#!/bin/bash

# Re-enable Mac sleep when Claude stops working

PIDS_DIR="/tmp/claude_working_pids"
LOCK_DIR="/tmp/claude_sleep.lock"

# Acquire lock
while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    sleep 0.1
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

# Remove this Claude's PID file
rm -f "$PIDS_DIR/$PPID"

# Count remaining
count=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')

# If none left, re-enable sleep
if [ "$count" -eq 0 ]; then
    sudo pmset -a disablesleep 0
fi

rmdir "$LOCK_DIR" 2>/dev/null
trap - EXIT
