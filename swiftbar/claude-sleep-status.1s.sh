#!/bin/bash

# SwiftBar plugin for Claude Code Sleep Preventer
# Detects working Claude instances by CPU usage (>10% = working)

# Count Claude processes actively using CPU (working)
# Threshold 10% - typing uses ~1-5%, actual work uses 15%+
working=$(ps aux | grep "[c]laude" | awk '$3 > 10.0 {count++} END {print count+0}')

# Count total Claude processes
running=$(pgrep -x "claude" 2>/dev/null | wc -l | tr -d ' ')

sleep_disabled=$(pmset -g | grep SleepDisabled | awk '{print $2}')

# Auto-manage sleep based on working status
if [ "$working" -gt 0 ]; then
    # Claude is working - ensure sleep is disabled
    if [ "$sleep_disabled" != "1" ]; then
        sudo pmset -a disablesleep 1 2>/dev/null
        sleep_disabled=1
    fi
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
    echo "Force Enable Sleep | bash='sudo pmset -a disablesleep 0' terminal=false refresh=true"
else
    # No Claude working - ensure sleep is enabled
    if [ "$sleep_disabled" = "1" ]; then
        sudo pmset -a disablesleep 0 2>/dev/null
        sleep_disabled=0
    fi
    if [ "$running" -gt 0 ]; then
        echo "💤 $running"
        echo "---"
        echo "Claude Code Sleep Preventer | color=gray"
        echo "$running open (idle)"
    else
        echo "😴"
        echo "---"
        echo "Claude Code Sleep Preventer | color=gray"
        echo "No Claude instances"
    fi
    echo "Sleep: Enabled | color=green"
fi
