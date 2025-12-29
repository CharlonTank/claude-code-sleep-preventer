#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Uses hooks for tracking + CPU check to clean up interrupted sessions

PIDS_DIR="/tmp/claude_working_pids"

# Grace period in seconds - don't clean up PIDs younger than this
GRACE_PERIOD=10

# Clean up stale PIDs:
# 1. Process doesn't exist anymore
# 2. Process exists but idle (CPU < 1%) AND older than grace period
if [ -d "$PIDS_DIR" ]; then
    now=$(date +%s)
    for pidfile in "$PIDS_DIR"/*; do
        [ -f "$pidfile" ] || continue
        pid=$(basename "$pidfile")

        # Check if process exists
        if ! ps -p "$pid" > /dev/null 2>&1; then
            rm -f "$pidfile"
            continue
        fi

        # Check file age - skip cleanup if too new (handles compacting, brief pauses)
        file_mtime=$(stat -f %m "$pidfile" 2>/dev/null)
        if [ -n "$file_mtime" ]; then
            age=$((now - file_mtime))
            if [ "$age" -lt "$GRACE_PERIOD" ]; then
                continue
            fi
        fi

        # Check if process is idle (CPU < 1%)
        cpu=$(ps -p "$pid" -o %cpu= 2>/dev/null | tr -d ' ')
        if [ -n "$cpu" ]; then
            idle=$(echo "$cpu < 1.0" | bc -l 2>/dev/null)
            if [ "$idle" = "1" ]; then
                rm -f "$pidfile"
            fi
        fi
    done
fi

# Count working instances
working=0
if [ -d "$PIDS_DIR" ]; then
    working=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')
fi

# Count total running
running=$(pgrep -x "claude" 2>/dev/null | wc -l | tr -d ' ')

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

# Auto-fix sleep state
if [ "$working" -gt 0 ] && [ "$sleep_disabled" != "1" ]; then
    sudo pmset -a disablesleep 1 2>/dev/null
    sleep_disabled=1
elif [ "$working" -eq 0 ] && [ "$sleep_disabled" = "1" ]; then
    sudo pmset -a disablesleep 0 2>/dev/null
    sleep_disabled=0
fi

if [ "$working" -gt 0 ]; then
    echo "☕ $working"
    echo "---"
    echo "Claude Code Sleep Preventer | color=green"
    echo "$working working / $running open | color=green"
    echo "Sleep: Disabled | color=orange"
    echo "---"
    thermal=$(pmset -g therm 2>/dev/null)
    if echo "$thermal" | grep -q "No thermal warning"; then
        echo "Thermal: OK | color=green"
    else
        echo "Thermal: Warning! | color=red"
    fi
    echo "---"
    echo "Force Enable Sleep | bash='sudo pmset -a disablesleep 0 && rm -rf /tmp/claude_working_pids' terminal=false refresh=true"
elif [ "$running" -gt 0 ]; then
    echo "💤 $running"
    echo "---"
    echo "Claude Code Sleep Preventer | color=gray"
    echo "$running open (idle)"
    echo "Sleep: Enabled | color=green"
else
    echo "😴"
    echo "---"
    echo "Claude Code Sleep Preventer | color=gray"
    echo "No Claude instances"
    echo "Sleep: Enabled | color=green"
fi
