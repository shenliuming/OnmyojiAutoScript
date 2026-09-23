use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use foster_protocol::AgentEvent;
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct AgentEventOutbox {
    queue: Arc<Mutex<VecDeque<AgentEvent>>>,
    notify: Arc<Notify>,
}

impl AgentEventOutbox {
    pub fn push(&self, event: AgentEvent) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_back(event);
            self.notify.notify_one();
        }
    }

    pub fn requeue_front(&self, event: AgentEvent) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_front(event);
            self.notify.notify_one();
        }
    }

    pub async fn recv(&self) -> AgentEvent {
        loop {
            let notified = self.notify.notified();

            if let Ok(mut queue) = self.queue.lock()
                && let Some(event) = queue.pop_front()
            {
                return event;
            }

            notified.await;
        }
    }

    pub fn len(&self) -> usize {
        self.queue.lock().map(|queue| queue.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
