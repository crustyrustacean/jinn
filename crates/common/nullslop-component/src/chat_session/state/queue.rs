//! Message queue methods for [`ChatSessionState`](super::ChatSessionState).

use std::collections::VecDeque;

use super::ChatSessionState;

impl ChatSessionState {
    /// Read-only access to the message queue.
    pub fn queue(&self) -> &VecDeque<String> {
        &self.core.message_queue
    }

    /// Number of messages waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.core.message_queue.len()
    }

    /// Push a message onto the back of the queue.
    pub fn enqueue_message(&mut self, text: String) {
        self.core.message_queue.push_back(text);
    }

    /// Pop the front message from the queue, if any.
    pub fn dequeue_message(&mut self) -> Option<String> {
        self.core.message_queue.pop_front()
    }

    /// Drain all queued messages, returning them in order.
    pub fn drain_queue(&mut self) -> VecDeque<String> {
        std::mem::take(&mut self.core.message_queue)
    }
}
