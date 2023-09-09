use std::{collections::BinaryHeap, cmp::Reverse};

use crate::proto::grapevine::RawMessage;
pub struct MinMaxHeap {
    heap: BinaryHeap<Reverse<RawMessage>>,
    max_id: Option<i64>,
}

impl MinMaxHeap {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            max_id: None,
        }
    }

    pub fn peek(&self) -> Option<&Reverse<RawMessage>> {
        self.heap.peek()
    }

    pub fn pop(&mut self) -> Option<Reverse<RawMessage>> {
        let msg = self.heap.pop();

        if self.heap.len() == 0 {
            self.max_id = None;
        }

        msg
    }

    pub fn push(&mut self, v: Reverse<RawMessage>) {
        let id = v.0.metadata.sequence;
        if let Some(max_id) = self.max_id.as_mut() {
            *max_id = id.max(*max_id);
        } else {
            self.max_id = Some(id);
        }

        self.heap.push(v);
    }

    /// it is valid only until heap does not contain duplicate messages
    pub fn has_gaps(&self) -> bool {
        self.count_missing() > 0
    }

    fn count_missing(&self) -> i64 {
        if let Some(max_id) = self.max_id {
            // unwrap: self.max_id is Some only if heap.len > 0
            let min_id = self.peek().unwrap().0.metadata.sequence;
            max_id - min_id + 1 - (self.heap.len() as i64)
        } else {
            0
        }
    }
}
