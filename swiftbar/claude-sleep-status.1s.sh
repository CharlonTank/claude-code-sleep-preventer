#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Refresh every 1 second

COUNTER_FILE="/tmp/claude_active_count"

# Get counter (working instances)
working=0
[ -f "$COUNTER_FILE" ] && working=$(cat "$COUNTER_FILE")

# Get actual running processes
running=$(pgrep -x "claude" 2>/dev/null | wc -l | tr -d ' ')

# If counter says working but no processes exist, reset counter
if [ "$working" -gt 0 ] && [ "$running" -eq 0 ]; then
    rm -f "$COUNTER_FILE"
    working=0
    sudo pmset -a disablesleep 0 2>/dev/null
fi

# Cap working at running (can't have more working than running)
if [ "$working" -gt "$running" ]; then
    echo "$running" > "$COUNTER_FILE"
    working=$running
fi

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

if [ "$working" -gt 0 ]; then
    # Claude is working
    echo "☕ $working"
    echo "---"
    echo "Claude Code Sleep Preventer | color=green"
    echo "$working working / $running running | color=green"
    if [ "$sleep_disabled" = "1" ]; then
        echo "Sleep: Disabled | color=orange"
    else
        echo "Sleep: Enabled (unexpected) | color=yellow"
    fi
    echo "---"
    thermal=$(pmset -g therm 2>/dev/null)
    if echo "$thermal" | grep -q "No thermal warning"; then
        echo "Thermal: OK | color=green"
    else
        echo "Thermal: Warning! | color=red"
    fi
    echo "---"
    echo "Force Enable Sleep | bash='sudo pmset -a disablesleep 0 && rm -f /tmp/claude_active_count' terminal=false refresh=true"
else
    # No Claude working
    if [ "$running" -gt 0 ]; then
        echo "💤 $running"
        echo "---"
        echo "Claude Code Sleep Preventer | color=gray"
        echo "$running idle (not working)"
    else
        echo "😴"
        echo "---"
        echo "Claude Code Sleep Preventer | color=gray"
        echo "No Claude instances"
    fi
    if [ "$sleep_disabled" = "1" ]; then
        echo "Sleep: Disabled (stale - fixing...) | color=red"
        sudo pmset -a disablesleep 0 2>/dev/null
    else
        echo "Sleep: Enabled | color=green"
    fi
fi
