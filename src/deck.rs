use std::time::Duration;

use elgato_streamdeck::{StreamDeck, StreamDeckInput, list_devices, new_hidapi};
use log::{debug, info};

use crate::actions;
use crate::config::{Action, Config};

pub enum InputResult {
    SwitchPage(String),
    NeewerCommand(String),
    CameraCommand(String),
    AudioCommand(String, i8),
    ActionFired(Action),
    LcdDoubleTap(u8),
    TimerCommand(String),
    /// Swipe direction: negative = right (previous page), positive = left (next page)
    SwipePage(i8),
}

pub fn connect() -> Result<StreamDeck, Box<dyn std::error::Error>> {
    let hidapi = new_hidapi()?;
    let devices = list_devices(&hidapi);

    if devices.is_empty() {
        return Err("No Stream Deck devices found".into());
    }

    let (kind, serial) = &devices[0];
    info!("Found device: {:?} (serial: {})", kind, serial);

    let deck = StreamDeck::connect(&hidapi, *kind, serial)?;
    Ok(deck)
}

fn handle_action(action: &Action) -> Option<InputResult> {
    match action {
        Action::SwitchPage { page } => Some(InputResult::SwitchPage(page.clone())),
        Action::Neewer { command, group } => {
            let key = match group {
                Some(g) => format!("{}:{}", command, g),
                None => command.clone(),
            };
            Some(InputResult::NeewerCommand(key))
        }
        Action::Camera { command } => Some(InputResult::CameraCommand(command.clone())),
        Action::Audio { command } => Some(InputResult::AudioCommand(command.clone(), 1)),
        Action::Sound { name } => {
            crate::soundboard::play_named(name);
            None
        }
        Action::Timer { command } => Some(InputResult::TimerCommand(command.clone())),
        Action::Multi { actions } => {
            // Execute all actions, return the last special result (page switch, etc.)
            let mut last_result = None;
            for action in actions {
                if let Some(r) = handle_action(action) {
                    last_result = Some(r);
                }
            }
            last_result
        }
        Action::LightPreset { brightness, temp_k, group } => {
            let key = match group {
                Some(g) => format!("preset:{}:{}:{}", brightness, temp_k, g),
                None => format!("preset:{}:{}", brightness, temp_k),
            };
            Some(InputResult::NeewerCommand(key))
        }
        other => {
            actions::execute(other);
            Some(InputResult::ActionFired(other.clone()))
        }
    }
}

/// State for double-tap detection on the LCD touchscreen
pub struct TouchState {
    last_tap_time: std::time::Instant,
    last_tap_segment: u8,
}

impl TouchState {
    pub fn new() -> Self {
        Self {
            last_tap_time: std::time::Instant::now() - std::time::Duration::from_secs(10),
            last_tap_segment: 255,
        }
    }
}

/// Polls for input and dispatches actions. Returns Some if a special action needs main loop handling.
/// Returns Err(()) if the device is disconnected (consecutive read failures).
pub fn poll_and_dispatch(
    deck: &StreamDeck,
    cfg: &Config,
    current_page: &str,
    error_count: &mut u32,
    touch: &mut TouchState,
) -> Result<Option<InputResult>, ()> {
    let input = match deck.read_input(Some(Duration::from_millis(50))) {
        Ok(input) => {
            *error_count = 0;
            input
        }
        Err(_) => {
            *error_count += 1;
            if *error_count > 20 {
                // 20 consecutive errors (~1 second) = device is gone
                log::error!("Stream Deck disconnected (20 consecutive read failures), exiting for restart");
                return Err(());
            }
            return Ok(None);
        }
    };

    let buttons = cfg.active_buttons(current_page);
    let encoders = cfg.active_encoders(current_page);

    let result = match input {
        StreamDeckInput::NoData => None,

        StreamDeckInput::ButtonStateChange(states) => {
            let mut out = None;
            for (i, pressed) in states.iter().enumerate() {
                if *pressed {
                    info!("Button {} pressed", i);
                    let key = i.to_string();
                    if let Some(button) = buttons.get(&key)
                        && let Some(action) = &button.on_press
                        && let Some(result) = handle_action(action)
                    {
                        out = Some(result);
                        break;
                    }
                }
            }
            out
        }

        StreamDeckInput::EncoderStateChange(states) => {
            let mut out = None;
            for (i, pressed) in states.iter().enumerate() {
                if *pressed {
                    info!("Encoder {} pressed", i);
                    let key = i.to_string();
                    if let Some(encoder) = encoders.get(&key)
                        && let Some(action) = &encoder.on_press
                        && let Some(result) = handle_action(action)
                    {
                        out = Some(result);
                        break;
                    }
                }
            }
            out
        }

        StreamDeckInput::EncoderTwist(values) => {
            let mut out = None;
            for (i, amount) in values.iter().enumerate() {
                if *amount != 0 {
                    let key = i.to_string();
                    debug!("Encoder {} twisted: {}", i, amount);
                    let action = encoders.get(&key).and_then(|enc| {
                        if *amount > 0 { enc.on_turn_cw.as_ref() } else { enc.on_turn_ccw.as_ref() }
                    });
                    if let Some(action) = action {
                        let mut result = handle_action(action);
                        if let Some(InputResult::AudioCommand(cmd, _)) = result {
                            result = Some(InputResult::AudioCommand(cmd, *amount));
                        }
                        if result.is_some() {
                            out = result;
                            break;
                        }
                    }
                }
            }
            out
        }

        StreamDeckInput::TouchScreenPress(x, _y) => {
            // Map x to LCD segment (0-3), each segment is 200px
            let segment = (x / 200).min(3) as u8;
            let now = std::time::Instant::now();
            let double_tap_window = std::time::Duration::from_millis(400);

            if segment == touch.last_tap_segment
                && now.duration_since(touch.last_tap_time) < double_tap_window
            {
                info!("LCD double-tap on segment {}", segment);
                touch.last_tap_segment = 255; // reset to prevent triple-tap
                Some(InputResult::LcdDoubleTap(segment))
            } else {
                touch.last_tap_time = now;
                touch.last_tap_segment = segment;
                None
            }
        }

        StreamDeckInput::TouchScreenLongPress(x, y) => {
            info!("Touch long press at ({}, {})", x, y);
            None
        }

        StreamDeckInput::TouchScreenSwipe(from, to) => {
            let dx = to.0 as i32 - from.0 as i32;
            if dx.abs() > 100 {
                // Swipe left (dx < 0) = next page, swipe right (dx > 0) = previous
                let direction: i8 = if dx < 0 { 1 } else { -1 };
                info!("LCD swipe {} (dx={})", if direction > 0 { "left→next" } else { "right→prev" }, dx);
                Some(InputResult::SwipePage(direction))
            } else {
                None
            }
        }
    };

    Ok(result)
}
