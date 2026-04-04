#!/bin/bash
# Enable Do Not Disturb
defaults -currentHost write com.apple.notificationcenterui doNotDisturb -bool true
killall NotificationCenter 2>/dev/null || true
