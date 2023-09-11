use std::{collections::{BinaryHeap, HashMap}, cmp::Reverse};

use crate::proto::grapevine::RawMessage;
pub struct MinMaxHeap {
    heap: BinaryHeap<Reverse<RawMessage>>,
    max_id: Option<i64>,
    counter: HashMap<i64, usize>,
}

impl MinMaxHeap {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            max_id: None,
            counter: HashMap::new(),
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

        if let Some(msg) = &msg {
            let id = msg.0.metadata.sequence;

            // unwrap: heap and counter must have 1:1 mapping
            let count = self.counter.get_mut(&id).unwrap();
            *count -= 1;
            if *count == 0 {
                self.counter.remove(&id);
            }
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

        self.counter.entry(id).and_modify(|v| *v += 1).or_insert(1);
        self.heap.push(v);
    }

    pub fn has_gaps(&self) -> bool {
        self.count_missing() > 0
    }

    fn count_missing(&self) -> i64 {
        if let Some(max_id) = self.max_id {
            // unwrap: self.max_id is Some only if heap.len > 0
            let min_id = self.peek().unwrap().0.metadata.sequence;
            let uniq = self.counter.len();
            max_id - min_id + 1 - (uniq as i64)
        } else {
            0
        }
    }
}
