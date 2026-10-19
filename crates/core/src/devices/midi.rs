//! Reassembles MIDI messages from a raw byte stream (running status,
//! interleaved realtime bytes and SysEx included).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// `channel` is 1–16.
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    /// Anything else with a known length (program change, pitch bend, …).
    Other,
}

#[derive(Debug, Default)]
pub struct Parser {
    status: Option<u8>,
    data: Vec<u8>,
    sysex: bool,
}

/// Number of data bytes of a channel voice message.
fn data_len(status: u8) -> usize {
    match status >> 4 {
        0xC | 0xD => 1,
        _ => 2,
    }
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Message> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte >= 0xF8 {
                continue; // realtime (clock, …), may appear anywhere
            }
            if byte & 0x80 != 0 {
                self.data.clear();
                match byte {
                    0xF0 => {
                        self.sysex = true;
                        self.status = None;
                    }
                    0xF7 => {
                        self.sysex = false;
                        self.status = None;
                    }
                    0xF1..=0xF6 => {
                        // System common: not needed, drop and ignore its data.
                        self.sysex = false;
                        self.status = None;
                    }
                    _ => {
                        self.sysex = false;
                        self.status = Some(byte);
                    }
                }
                continue;
            }
            if self.sysex {
                continue;
            }
            let Some(status) = self.status else { continue };
            self.data.push(byte);
            if self.data.len() == data_len(status) {
                out.push(message(status, &self.data));
                self.data.clear(); // keep the status: running status
            }
        }
        out
    }
}

fn message(status: u8, data: &[u8]) -> Message {
    let channel = (status & 0x0F) + 1;
    match (status >> 4, data) {
        (0xB, &[controller, value]) => Message::ControlChange {
            channel,
            controller,
            value,
        },
        (0x9, &[note, 0]) => Message::NoteOff {
            channel,
            note,
            velocity: 0,
        },
        (0x9, &[note, velocity]) => Message::NoteOn {
            channel,
            note,
            velocity,
        },
        (0x8, &[note, velocity]) => Message::NoteOff {
            channel,
            note,
            velocity,
        },
        _ => Message::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(controller: u8, value: u8) -> Message {
        Message::ControlChange {
            channel: 12,
            controller,
            value,
        }
    }

    #[test]
    fn control_change() {
        assert_eq!(Parser::new().feed(&[0xbb, 0x51, 0x7f]), vec![cc(81, 127)]);
    }

    #[test]
    fn running_status_and_split_chunks() {
        let mut p = Parser::new();
        assert_eq!(p.feed(&[0xbb, 0x50]), vec![]);
        assert_eq!(p.feed(&[0x43, 0x50, 0x42]), vec![cc(80, 67), cc(80, 66)]);
    }

    #[test]
    fn realtime_and_sysex_are_skipped() {
        let mut p = Parser::new();
        let bytes = [
            0xbb, 0xf8, 0x52, 0xfe, 0x7f, 0xf0, 0x01, 0x02, 0xf7, 0xbb, 0x52, 0x00,
        ];
        assert_eq!(p.feed(&bytes), vec![cc(82, 127), cc(82, 0)]);
    }

    #[test]
    fn data_without_status_is_dropped() {
        assert_eq!(Parser::new().feed(&[0x10, 0x20]), vec![]);
    }

    #[test]
    fn notes() {
        let mut p = Parser::new();
        assert_eq!(
            p.feed(&[0x90, 60, 100, 60, 0, 0x80, 61, 5]),
            vec![
                Message::NoteOn {
                    channel: 1,
                    note: 60,
                    velocity: 100
                },
                Message::NoteOff {
                    channel: 1,
                    note: 60,
                    velocity: 0
                },
                Message::NoteOff {
                    channel: 1,
                    note: 61,
                    velocity: 5
                },
            ]
        );
    }

    #[test]
    fn short_messages() {
        assert_eq!(Parser::new().feed(&[0xc0, 5]), vec![Message::Other]);
    }
}
