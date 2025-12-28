#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Refresh every 1 second

COUNTER_FILE="/tmp/claude_active_count"
count=0
[ -f "$COUNTER_FILE" ] && count=$(cat "$COUNTER_FILE")

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

if [ "$count" -gt 0 ] && [ "$sleep_disabled" = "1" ]; then
    # Claude is running, sleep disabled
    if [ "$count" -eq 1 ]; then
        echo "☕ 1"
    else
        echo "☕ $count"
    fi
    echo "---"
    echo "Claude Code Sleep Preventer | color=green"
    echo "$count Claude instance(s) active | color=green"
    echo "Sleep: Disabled | color=orange"
    echo "---"
    # Check thermal
    thermal=$(pmset -g therm 2>/dev/null)
    if echo "$thermal" | grep -q "No thermal warning"; then
        echo "Thermal: OK | color=green"
    else
        echo "Thermal: Warning! | color=red"
    fi
    echo "---"
    echo "Force Enable Sleep | bash='sudo pmset -a disablesleep 0 && rm -f /tmp/claude_active_count' terminal=false refresh=true"
else
    # No Claude running or sleep enabled
    echo "😴"
    echo "---"
    echo "Claude Code Sleep Preventer | color=gray"
    echo "No Claude instances active"
    echo "Sleep: Enabled | color=green"
fi
