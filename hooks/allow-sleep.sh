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

# Count remaining active PIDs
count=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')

# If no more working instances, re-enable sleep
if [ "$count" -eq 0 ]; then
    # Kill thermal monitor
    if [ -f /tmp/thermal_monitor.pid ]; then
        kill $(cat /tmp/thermal_monitor.pid) 2>/dev/null
        rm /tmp/thermal_monitor.pid
    fi

    sudo pmset -a disablesleep 0
fi

rmdir "$LOCK_DIR" 2>/dev/null
trap - EXIT
