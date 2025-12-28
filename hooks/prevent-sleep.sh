#!/bin/bash

# Prevent Mac from sleeping while Claude Code is running
# Supports multiple Claude instances

COUNTER_FILE="/tmp/claude_active_count"
LOCK_DIR="/tmp/claude_sleep.lock"

# Acquire lock (mkdir is atomic)
while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    sleep 0.1
done
trap 'rmdir "$LOCK_DIR" 2>/dev/null' EXIT

# Increment counter
count=0
[ -f "$COUNTER_FILE" ] && count=$(cat "$COUNTER_FILE")
count=$((count + 1))
echo "$count" > "$COUNTER_FILE"

# If first Claude instance, set up sleep prevention
if [ "$count" -eq 1 ]; then
    # Disable sleep completely (works with lid closed, on battery)
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
