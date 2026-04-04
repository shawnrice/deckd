#!/bin/bash
# Send a notification to deckd's LCD strip
# Usage: deckd-notify.sh "message text"
#
# For Claude Code hooks, add to .claude/settings.json:
# {
#   "hooks": {
#     "PostToolUse": [{
#       "matcher": "Write|Edit",
#       "command": "/Users/shawn/projects/deckd/scripts/deckd-notify.sh 'Claude wrote code'"
#     }],
#     "Stop": [{
#       "command": "/Users/shawn/projects/deckd/scripts/deckd-notify.sh 'Claude finished'"
#     }]
#   }
# }

MSG="${1:-notification}"
echo -n "$MSG" | nc -u -w0 127.0.0.1 9876 2>/dev/null
