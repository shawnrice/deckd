#!/bin/bash
# Generate/copy all assets that aren't tracked in git.
# Run this after cloning: ./scripts/setup-assets.sh

set -e
cd "$(dirname "$0")/.."

ASSETS="assets"
ICONS="$ASSETS/icons"
SOUNDS="$ASSETS/sounds"

mkdir -p "$ICONS" "$SOUNDS"

# ── Fonts ──────────────────────────────────────────────────────
echo "Setting up fonts..."
FONT_SRC="$HOME/Library/Fonts/BerkeleyMono-Regular.ttf"
FONT_BOLD_SRC="$HOME/Library/Fonts/BerkeleyMono-Bold.ttf"

if [ -f "$FONT_SRC" ]; then
    cp "$FONT_SRC" "$ASSETS/font.ttf"
    cp "$FONT_BOLD_SRC" "$ASSETS/font-bold.ttf"
    echo "  Copied Berkeley Mono"
else
    # Fallback to Courier New
    cp "/System/Library/Fonts/Supplemental/Courier New.ttf" "$ASSETS/font.ttf"
    cp "/System/Library/Fonts/Supplemental/Courier New Bold.ttf" "$ASSETS/font-bold.ttf" 2>/dev/null || cp "$ASSETS/font.ttf" "$ASSETS/font-bold.ttf"
    echo "  Berkeley Mono not found, using Courier New"
fi

# ── App icons ──────────────────────────────────────────────────
echo "Extracting app icons..."
extract_icon() {
    local app="$1" name="$2"
    local icns=$(defaults read "$app/Contents/Info" CFBundleIconFile 2>/dev/null | sed 's/\.icns$//')
    [ -z "$icns" ] && icns=$(defaults read "$app/Contents/Info" CFBundleIconName 2>/dev/null)
    local path="$app/Contents/Resources/${icns}.icns"
    [ ! -f "$path" ] && path=$(find "$app/Contents/Resources" -name "*.icns" -maxdepth 1 2>/dev/null | head -1)
    if [ -f "$path" ]; then
        sips -s format png -z 120 120 "$path" --out "$ICONS/${name}.png" 2>/dev/null
        echo "  $name"
    fi
}
extract_icon "/Applications/Ghostty.app" "ghostty"
extract_icon "/Applications/Slack.app" "slack"
extract_icon "/Applications/Google Chrome.app" "chrome"
[ -d "$HOME/Applications/Brain.fm.app" ] && extract_icon "$HOME/Applications/Brain.fm.app" "brainfm"

# ── Emoji icons (ship page) ────────────────────────────────────
echo "Rendering emoji icons..."
render_emoji() {
    local name="$1" emoji="$2"
    swift -e "
import AppKit
let size = NSSize(width: 96, height: 96)
let img = NSImage(size: size)
img.lockFocus()
NSColor.clear.setFill()
NSRect(origin: .zero, size: size).fill()
let attrs: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 72)]
let str = NSAttributedString(string: \"$emoji\", attributes: attrs)
let strSize = str.size()
str.draw(at: NSPoint(x: (size.width - strSize.width) / 2, y: (size.height - strSize.height) / 2))
img.unlockFocus()
let tiff = img.tiffRepresentation!
let bitmap = NSBitmapImageRep(data: tiff)!
let png = bitmap.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: \"$ICONS/$name.png\"))
" 2>/dev/null && echo "  $name"
}
render_emoji "wave" "🌊"
render_emoji "hug" "🤗"
render_emoji "carousel" "🎠"
render_emoji "robot" "🤖"
render_emoji "monkey" "🙊"
render_emoji "alien" "👾"
render_emoji "siren" "🚨"
render_emoji "unicorn" "🦄"

# ── SF Symbol icons ────────────────────────────────────────────
echo "Rendering SF Symbol icons..."
render_sf() {
    local name="$1" symbol="$2" r="$3" g="$4" b="$5"
    swift -e "
import AppKit
let size = NSSize(width: 120, height: 120)
let img = NSImage(size: size)
img.lockFocus()
NSColor.clear.setFill()
NSRect(origin: .zero, size: size).fill()
if let symbol = NSImage(systemSymbolName: \"$symbol\", accessibilityDescription: nil) {
    let config = NSImage.SymbolConfiguration(pointSize: 56, weight: .medium)
    let configured = symbol.withSymbolConfiguration(config)!
    let color = NSColor(red: $r/255.0, green: $g/255.0, blue: $b/255.0, alpha: 1.0)
    let tinted = configured.copy() as! NSImage
    tinted.lockFocus()
    color.set()
    NSRect(origin: .zero, size: tinted.size).fill(using: .sourceAtop)
    tinted.unlockFocus()
    let origin = NSPoint(x: (size.width - tinted.size.width) / 2, y: (size.height - tinted.size.height) / 2)
    tinted.draw(at: origin, from: .zero, operation: .sourceOver, fraction: 1.0)
}
img.unlockFocus()
let tiff = img.tiffRepresentation!
let bitmap = NSBitmapImageRep(data: tiff)!
let png = bitmap.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: \"$ICONS/$name.png\"))
" 2>/dev/null && echo "  $name"
}
render_sf "mic" "mic.fill" 255 102 102
render_sf "mic_mute" "mic.slash.fill" 255 68 68
render_sf "tasks" "checklist" 240 192 64
render_sf "git" "arrow.triangle.branch" 76 201 240
render_sf "eye" "eye.fill" 240 144 112
render_sf "music" "music.note" 123 104 238
render_sf "right_arrow" "forward.fill" 123 104 238
render_sf "rocket" "paperplane.fill" 76 201 240
render_sf "grid" "square.grid.2x2.fill" 136 136 153
render_sf "sun" "sun.max.fill" 255 255 102
render_sf "moon" "moon.fill" 224 192 255
render_sf "camera" "camera.fill" 112 184 255
render_sf "link" "link" 224 224 224
render_sf "back" "arrow.uturn.backward" 136 136 153
render_sf "power" "power" 102 255 102
render_sf "refresh" "arrow.counterclockwise" 240 144 112
render_sf "focus" "scope" 192 224 255
render_sf "af" "camera.metering.center.weighted" 102 255 102
render_sf "pan_left" "chevron.left" 112 184 255
render_sf "pan_right" "chevron.right" 112 184 255
render_sf "tilt_up" "chevron.up" 112 184 255
render_sf "tilt_down" "chevron.down" 112 184 255
render_sf "zoom_in" "plus.magnifyingglass" 112 184 255

# ── Sounds ─────────────────────────────────────────────────────
echo "Copying system sounds..."
for snd in Basso Blow Bottle Frog Funk Glass Hero Morse Ping Pop Purr Sosumi Submarine Tink; do
    lower=$(echo "$snd" | tr 'A-Z' 'a-z')
    cp "/System/Library/Sounds/${snd}.aiff" "$SOUNDS/${lower}.aiff" 2>/dev/null
done
echo "  Copied $(ls "$SOUNDS"/*.aiff 2>/dev/null | wc -l | tr -d ' ') system sounds"

# Generate synth sounds
echo "Generating sounds..."
python3 -c "
import struct, wave, math
RATE = 44100
def tone(freq, dur, vol=0.5):
    return [vol * min(1, 1 - max(0, (i - int(RATE*dur*0.7)) / (int(RATE*dur*0.3)))) * math.sin(2 * math.pi * freq * i / RATE) for i in range(int(RATE * dur))]
def write_wav(path, samples):
    with wave.open(path, 'w') as f:
        f.setnchannels(1); f.setsampwidth(2); f.setframerate(RATE)
        for s in samples: f.writeframes(struct.pack('<h', int(max(-1, min(1, s)) * 32767)))
write_wav('$SOUNDS/success.wav', tone(523, 0.15, 0.4) + tone(659, 0.15, 0.4) + tone(784, 0.3, 0.5))
write_wav('$SOUNDS/timer_done.wav', tone(880, 0.3, 0.3) + [0]*int(RATE*0.1) + tone(880, 0.3, 0.3) + [0]*int(RATE*0.1) + tone(1047, 0.5, 0.4))
" 2>/dev/null
echo "  Generated success + timer_done"

# Download real sounds
echo "Downloading sounds..."
curl -sL "https://orangefreesounds.com/wp-content/uploads/2022/04/Sad-trombone.mp3" -o "$SOUNDS/sad_trombone.mp3" 2>/dev/null && echo "  sad_trombone"
curl -sL "https://www.myinstants.com/media/sounds/crickets.mp3" -o "$SOUNDS/crickets_full.mp3" 2>/dev/null
if [ -f "$SOUNDS/crickets_full.mp3" ]; then
    ffmpeg -y -i "$SOUNDS/crickets_full.mp3" -t 1.5 -af "afade=t=out:st=1:d=0.5" "$SOUNDS/crickets.mp3" 2>/dev/null && echo "  crickets"
    rm "$SOUNDS/crickets_full.mp3"
fi
curl -sL "https://www.myinstants.com/media/sounds/sitcom-laughing.mp3" -o "$SOUNDS/laugh_full.mp3" 2>/dev/null
if [ -f "$SOUNDS/laugh_full.mp3" ]; then
    ffmpeg -y -i "$SOUNDS/laugh_full.mp3" -t 2 -af "afade=t=out:st=1.5:d=0.5" "$SOUNDS/laugh.mp3" 2>/dev/null && echo "  laugh"
    rm "$SOUNDS/laugh_full.mp3"
fi

echo ""
echo "Done! Assets ready at $ASSETS/"
echo "Icons: $(ls "$ICONS"/*.png 2>/dev/null | wc -l | tr -d ' ')"
echo "Sounds: $(ls "$SOUNDS"/* 2>/dev/null | wc -l | tr -d ' ')"
