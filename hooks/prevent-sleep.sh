#!/bin/bash

# Prevent Mac from sleeping while Claude Code is running
# Tracks by PID for accurate counting even with interrupts

PIDS_DIR="/tmp/claude_working_pids"
LOCK_DIR="/tmp/claude_sleep.lock"

# Acquire lock
while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    sleep 0.1
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

# Create PIDs directory
mkdir -p "$PIDS_DIR"

# Register this Claude's parent PID (the claude process)
# PPID is the parent of this script, which is the claude process
echo "$$" > "$PIDS_DIR/$PPID"

# Count active PIDs
count=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')

# If first working instance, enable sleep prevention
if [ "$count" -eq 1 ]; then
    sudo pmset -a disablesleep 1

    # Start thermal monitor
    if [ -f /tmp/thermal_monitor.pid ]; then
        kill $(cat /tmp/thermal_monitor.pid) 2>/dev/null
        rm -f /tmp/thermal_monitor.pid
    fi
    nohup "$HOME/.claude/hooks/thermal-monitor.sh" > /dev/null 2>&1 &
fi

rmdir "$LOCK_DIR" 2>/dev/null
trap - EXIT
