#!/bin/bash

# Re-enable Mac sleep only when ALL Claude instances have stopped

COUNTER_FILE="/tmp/claude_active_count"
LOCK_DIR="/tmp/claude_sleep.lock"

# Acquire lock (mkdir is atomic)
while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    sleep 0.1
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

# Decrement counter
count=0
[ -f "$COUNTER_FILE" ] && count=$(cat "$COUNTER_FILE")
count=$((count - 1))
[ "$count" -lt 0 ] && count=0
echo "$count" > "$COUNTER_FILE"

# If no more Claude instances, re-enable sleep
if [ "$count" -eq 0 ]; then
    # Kill caffeinate
    if [ -f /tmp/claude_caffeinate.pid ]; then
        kill $(cat /tmp/claude_caffeinate.pid) 2>/dev/null
        rm /tmp/claude_caffeinate.pid
    fi

    # Kill thermal monitor
    if [ -f /tmp/thermal_monitor.pid ]; then
        kill $(cat /tmp/thermal_monitor.pid) 2>/dev/null
        rm /tmp/thermal_monitor.pid
    fi

    # Re-enable sleep
    sudo pmset -a disablesleep 0

    # Clean up counter file
    rm -f "$COUNTER_FILE"
fi

rmdir "$LOCK_DIR" 2>/dev/null
trap - EXIT
