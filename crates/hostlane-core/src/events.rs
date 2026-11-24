//! In-process event bus feeding `events.subscribe` connections.

use hostlane_protocol::Event;
use tokio::sync::broadcast;

/// Events buffered per subscriber before it is told it lagged.
const CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            sender: broadcast::channel(CAPACITY).0,
        }
    }
}

impl EventBus {
    pub fn publish(&self, event: Event) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}
