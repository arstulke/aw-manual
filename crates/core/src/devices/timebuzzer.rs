//! timeBuzzer: a class-compliant USB MIDI device. Everything arrives as
//! Control Change messages:
//!
//! | CC | Meaning |
//! |---|---|
//! | 82 | button: 127 = pressed, 0 = released |
//! | 80 | turning: a running position, ±1 per step (rising = right) |
//! | 81 | proximity sensor: 0 = a hand hovers above, 127 = idle (repeated while it stays) |
//!
//! These are protocol constants of the device family, not identifiers of a
//! particular unit; the unit is selected in the configuration.

use super::midi::{Message, Parser};
use super::{Decoder, DeviceAction, Driver, InputInfo, Signal, Transport};
use crate::ipc::NavCommand;

const CC_TURN: u8 = 80;
const CC_BUTTON: u8 = 82;
const CC_PROXIMITY: u8 = 81;

pub struct TimeBuzzer;

impl Driver for TimeBuzzer {
    fn id(&self) -> &'static str {
        "timebuzzer"
    }

    fn title(&self) -> &'static str {
        "timeBuzzer"
    }

    fn transport(&self) -> Transport {
        Transport::Midi
    }

    fn inputs(&self) -> &'static [InputInfo] {
        &[
            InputInfo {
                name: "press",
                description: "short press",
            },
            InputInfo {
                name: "long_press",
                description: "press held for devices.timebuzzer.long_press",
            },
            InputInfo {
                name: "turn_left",
                description: "one step to the left",
            },
            InputInfo {
                name: "turn_right",
                description: "one step to the right",
            },
            InputInfo {
                name: "hover",
                description: "a hand approaches the buzzer",
            },
        ]
    }

    /// Operates the popover in scroll mode: press opens/confirms, turning
    /// navigates, a long press stops the activity.
    fn default_bindings(&self) -> Vec<(&'static str, DeviceAction)> {
        vec![
            ("press", DeviceAction::Popover),
            ("long_press", DeviceAction::Stop),
            ("turn_left", DeviceAction::Nav(NavCommand::Prev)),
            ("turn_right", DeviceAction::Nav(NavCommand::Next)),
            ("hover", DeviceAction::OpenPopover),
        ]
    }

    fn decoder(&self) -> Box<dyn Decoder> {
        Box::new(TimeBuzzerDecoder::default())
    }
}

#[derive(Default)]
pub struct TimeBuzzerDecoder {
    parser: Parser,
    turn: Option<u8>,
    near: Option<bool>,
}

impl Decoder for TimeBuzzerDecoder {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Signal> {
        let mut out = Vec::new();
        for msg in self.parser.feed(bytes) {
            let Message::ControlChange {
                controller, value, ..
            } = msg
            else {
                continue;
            };
            match controller {
                CC_BUTTON => out.push(Signal::Button {
                    name: "press",
                    down: value > 0,
                }),
                CC_PROXIMITY => {
                    // The device repeats its state; report only the approach.
                    // A hand that is already there at connect time is not one.
                    let near = value == 0;
                    if self.near.replace(near) == Some(false) && near {
                        out.push(Signal::Trigger("hover"));
                    }
                }
                CC_TURN => {
                    let last = self.turn.replace(value);
                    // The first value only tells the position.
                    let Some(last) = last else { continue };
                    let mut delta = i16::from(value) - i16::from(last);
                    // The position may wrap around at the ends of its range.
                    if delta > 64 {
                        delta -= 128;
                    } else if delta < -64 {
                        delta += 128;
                    }
                    match delta {
                        0 => {}
                        d if d > 0 => out.push(Signal::Trigger("turn_right")),
                        _ => out.push(Signal::Trigger("turn_left")),
                    }
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(d: &mut TimeBuzzerDecoder, bytes: &[u8]) -> Vec<Signal> {
        d.feed(bytes)
    }

    #[test]
    fn button() {
        let mut d = TimeBuzzerDecoder::default();
        assert_eq!(
            feed(&mut d, &[0xbb, 0x52, 0x7f, 0xbb, 0x52, 0x00]),
            vec![
                Signal::Button {
                    name: "press",
                    down: true
                },
                Signal::Button {
                    name: "press",
                    down: false
                },
            ]
        );
    }

    #[test]
    fn turning_uses_the_direction_of_the_position() {
        let mut d = TimeBuzzerDecoder::default();
        // 67 → 66 → 65 (left), then 66 (right); the first value only sets the position.
        let signals = feed(
            &mut d,
            &[
                0xbb, 0x50, 67, 0xbb, 0x50, 66, 0xbb, 0x50, 65, 0xbb, 0x50, 66,
            ],
        );
        assert_eq!(
            signals,
            vec![
                Signal::Trigger("turn_left"),
                Signal::Trigger("turn_left"),
                Signal::Trigger("turn_right"),
            ]
        );
    }

    #[test]
    fn wrap_around() {
        let mut d = TimeBuzzerDecoder::default();
        assert_eq!(
            feed(&mut d, &[0xbb, 0x50, 127, 0xbb, 0x50, 0, 0xbb, 0x50, 127]),
            vec![Signal::Trigger("turn_right"), Signal::Trigger("turn_left")]
        );
    }

    #[test]
    fn hover_fires_once_per_approach() {
        let mut d = TimeBuzzerDecoder::default();
        // Idle (repeated), approach (repeated), leave, approach again.
        let bytes = [
            0xbb, 0x51, 0x7f, 0xbb, 0x51, 0x7f, 0xbb, 0x51, 0x00, 0xbb, 0x51, 0x00, 0xbb, 0x51,
            0x7f, 0xbb, 0x51, 0x00,
        ];
        assert_eq!(
            feed(&mut d, &bytes),
            vec![Signal::Trigger("hover"), Signal::Trigger("hover")]
        );
    }

    #[test]
    fn hand_already_there_at_connect_is_not_an_approach() {
        let mut d = TimeBuzzerDecoder::default();
        assert_eq!(feed(&mut d, &[0xbb, 0x51, 0x00, 0xbb, 0x51, 0x00]), vec![]);
    }

    #[test]
    fn default_bindings_cover_all_inputs() {
        let names: Vec<_> = TimeBuzzer
            .default_bindings()
            .iter()
            .map(|(n, _)| *n)
            .collect();
        let inputs: Vec<_> = TimeBuzzer.inputs().iter().map(|i| i.name).collect();
        assert_eq!(names, inputs);
    }
}
