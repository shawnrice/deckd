#!/bin/bash
# Disable Do Not Disturb
defaults -currentHost write com.apple.notificationcenterui doNotDisturb -bool false
killall NotificationCenter 2>/dev/null || true
