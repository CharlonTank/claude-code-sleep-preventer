#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Refresh every 1 second

PIDS_DIR="/tmp/claude_working_pids"

# Clean up stale PIDs (process no longer exists)
if [ -d "$PIDS_DIR" ]; then
    for pidfile in "$PIDS_DIR"/*; do
        [ -f "$pidfile" ] || continue
        pid=$(basename "$pidfile")
        if ! ps -p "$pid" > /dev/null 2>&1; then
            rm -f "$pidfile"
        fi
    done
fi

# Count working instances (valid PID files)
working=0
if [ -d "$PIDS_DIR" ]; then
    working=$(ls -1 "$PIDS_DIR" 2>/dev/null | wc -l | tr -d ' ')
fi

# Get actual running processes
running=$(pgrep -x "claude" 2>/dev/null | wc -l | tr -d ' ')

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

# Auto-fix: if no working but sleep disabled, re-enable sleep
if [ "$working" -eq 0 ] && [ "$sleep_disabled" = "1" ]; then
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
