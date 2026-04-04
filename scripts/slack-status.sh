#!/bin/bash
# Set Slack status via UI automation, then return focus to the previous app
# Usage: slack-status.sh "status text"
# Usage: slack-status.sh --clear

STATUS="$1"

if [ "$STATUS" = "--clear" ]; then
    osascript << 'EOF'
        set prevApp to name of (info for (path to frontmost application))
        tell application "Slack" to activate
        delay 0.3
        tell application "System Events"
            keystroke "y" using {command down, shift down}
            delay 0.5
            keystroke "a" using {command down}
            delay 0.1
            key code 51
            delay 0.1
            key code 36
        end tell
        delay 0.3
        tell application prevApp to activate
EOF
else
    osascript << EOF
        set prevApp to name of (info for (path to frontmost application))
        tell application "Slack" to activate
        delay 0.3
        tell application "System Events"
            keystroke "y" using {command down, shift down}
            delay 0.5
            keystroke "a" using {command down}
            delay 0.1
            key code 51
            delay 0.1
            keystroke "$STATUS"
            delay 0.2
            key code 36
        end tell
        delay 0.3
        tell application prevApp to activate
EOF
fi
