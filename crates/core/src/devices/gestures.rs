//! Turns button down/up signals into `<name>` and `long_<name>` inputs.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::Signal;

/// Decides between a short and a long press when a button is released, so the
/// result only depends on the timestamps of the signals.
pub struct Gestures {
    long_press: Duration,
    down: HashMap<&'static str, Instant>,
}

impl Gestures {
    pub fn new(long_press: Duration) -> Self {
        Gestures {
            long_press,
            down: HashMap::new(),
        }
    }

    /// Returns the input that the signal completes, if any.
    pub fn handle(&mut self, signal: Signal, at: Instant) -> Option<String> {
        match signal {
            Signal::Trigger(name) => Some(name.to_string()),
            Signal::Button { name, down: true } => {
                self.down.insert(name, at);
                None
            }
            Signal::Button { name, down: false } => {
                let since = self.down.remove(name)?;
                Some(if at.saturating_duration_since(since) >= self.long_press {
                    format!("long_{name}")
                } else {
                    name.to_string()
                })
            }
        }
    }

    /// Forgets pressed buttons (after a reconnect, their release is lost).
    pub fn reset(&mut self) {
        self.down.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(down: bool) -> Signal {
        Signal::Button {
            name: "press",
            down,
        }
    }

    #[test]
    fn short_and_long() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut g = Gestures::new(Duration::from_millis(600));
        assert_eq!(g.handle(button(true), ms(0)), None);
        assert_eq!(g.handle(button(false), ms(599)), Some("press".into()));
        assert_eq!(g.handle(button(true), ms(1000)), None);
        assert_eq!(g.handle(button(false), ms(1600)), Some("long_press".into()));
    }

    #[test]
    fn release_without_press_is_ignored() {
        let mut g = Gestures::new(Duration::from_millis(600));
        assert_eq!(g.handle(button(false), Instant::now()), None);
    }

    #[test]
    fn triggers_pass_through_and_reset_forgets_buttons() {
        let t0 = Instant::now();
        let mut g = Gestures::new(Duration::from_millis(600));
        assert_eq!(
            g.handle(Signal::Trigger("turn_left"), t0),
            Some("turn_left".into())
        );
        g.handle(button(true), t0);
        g.reset();
        assert_eq!(g.handle(button(false), t0), None);
    }
}
