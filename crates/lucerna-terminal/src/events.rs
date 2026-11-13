//! Events the terminal sends to the UI, and the batch the UI groups them into.

/// What a terminal tells the UI. Raw PTY bytes never go through this channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// New content is ready to draw.
    Wakeup,
    /// The app set the title (`Some`) or reset it (`None`).
    Title(Option<String>),
    Bell,
    /// The child exited with this code, or `None` when a signal killed it.
    Exited(Option<i32>),
    /// OSC 52 write: the app asks to put this text on the clipboard.
    ClipboardStore(String),
}

/// The most events one batch carries before the UI handles it (spec §5.1).
pub const MAX_BATCH_EVENTS: usize = 100;

/// Events gathered over one batch window. Wakeups collapse into one flag.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Batch {
    pub wakeup: bool,
    pub events: Vec<TerminalEvent>,
}

impl Batch {
    /// Adds an event and returns `true` once the batch is full.
    pub fn push(&mut self, ev: TerminalEvent) -> bool {
        match ev {
            TerminalEvent::Wakeup => self.wakeup = true,
            ev => self.events.push(ev),
        }
        self.events.len() >= MAX_BATCH_EVENTS
    }

    pub fn is_empty(&self) -> bool {
        !self.wakeup && self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_coalesces_wakeups() {
        let mut batch = Batch::default();
        assert!(batch.is_empty());
        for _ in 0..50 {
            assert!(!batch.push(TerminalEvent::Wakeup));
        }
        assert!(batch.wakeup);
        assert!(batch.events.is_empty());
        assert!(!batch.is_empty());
    }

    #[test]
    fn batch_is_full_at_100_events() {
        let mut batch = Batch::default();
        for i in 1..100 {
            assert!(!batch.push(TerminalEvent::Bell), "push {i} is not full yet");
        }
        assert!(batch.push(TerminalEvent::Bell));
        assert_eq!(batch.events.len(), 100);
    }

    #[test]
    fn exit_is_kept() {
        let mut batch = Batch::default();
        batch.push(TerminalEvent::Wakeup);
        batch.push(TerminalEvent::Exited(Some(3)));
        assert_eq!(batch.events, vec![TerminalEvent::Exited(Some(3))]);
        assert!(batch.wakeup);
    }
}
