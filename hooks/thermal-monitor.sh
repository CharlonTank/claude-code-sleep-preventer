#!/bin/bash

# Monitor thermal state and re-enable sleep if Mac gets too hot

PIDFILE="/tmp/thermal_monitor.pid"
COUNTER_FILE="/tmp/claude_active_count"

echo $$ > "$PIDFILE"

cleanup() {
    rm -f "$PIDFILE"
    exit 0
}
trap cleanup SIGTERM SIGINT

while true; do
    thermal_output=$(pmset -g therm 2>/dev/null)

    # Check for thermal or performance warnings
    if echo "$thermal_output" | grep -qE "(thermal warning level|performance warning level)" | grep -v "No "; then
        echo "$(date): Thermal warning detected, forcing sleep"

        # Reset counter
        rm -f "$COUNTER_FILE"

        # Re-enable sleep
        sudo pmset -a disablesleep 0

        cleanup
    fi

    sleep 30
done
