#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Refresh every 1 second

# Count actual Claude CLI processes (more reliable than counter file)
count=$(pgrep -x "claude" 2>/dev/null | wc -l | tr -d ' ')

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

if [ "$count" -gt 0 ]; then
    # Claude is running
    if [ "$count" -eq 1 ]; then
        echo "☕ 1"
    else
        echo "☕ $count"
    fi
    echo "---"
    echo "Claude Code Sleep Preventer | color=green"
    echo "$count Claude instance(s) running | color=green"
    if [ "$sleep_disabled" = "1" ]; then
        echo "Sleep: Disabled | color=orange"
    else
        echo "Sleep: Enabled (hook may not have fired) | color=yellow"
    fi
    echo "---"
    # Check thermal
    thermal=$(pmset -g therm 2>/dev/null)
    if echo "$thermal" | grep -q "No thermal warning"; then
        echo "Thermal: OK | color=green"
    else
        echo "Thermal: Warning! | color=red"
    fi
    echo "---"
    echo "Force Enable Sleep | bash='sudo pmset -a disablesleep 0' terminal=false refresh=true"
else
    # No Claude running
    echo "😴"
    echo "---"
    echo "Claude Code Sleep Preventer | color=gray"
    echo "No Claude instances running"
    if [ "$sleep_disabled" = "1" ]; then
        echo "Sleep: Disabled (stale - fixing...) | color=red"
        # Auto-fix stale state
        sudo pmset -a disablesleep 0 2>/dev/null
    else
        echo "Sleep: Enabled | color=green"
    fi
fi
