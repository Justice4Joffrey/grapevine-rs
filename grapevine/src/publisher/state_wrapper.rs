/// Internal type to separate internals from the publisher. Wraps a state
/// type S along with the latest sequence for this publisher.
#[derive(Debug, Default)]
pub struct PublisherStateWrapper<S> {
    state: S,
    sequence: i64,
}

impl<S> PublisherStateWrapper<S> {
    /// Return the current sequence and increment
    pub fn stamp(&mut self) -> i64 {
        let seq = self.sequence;
        self.sequence += 1;
        seq
    }

    /// Get a mutable reference to the inner state
    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    /// Get a reference to the inner state
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Get a reference to the inner state
    pub fn sequence(&self) -> i64 {
        self.sequence
    }
}

impl<S: Default> PublisherStateWrapper<S> {
    /// Create an empty state with the given sequence.
    pub fn with_sequence(sequence: i64) -> Self {
        Self {
            state: S::default(),
            sequence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stamp() {
        let mut state_wrapper = PublisherStateWrapper {
            state: (),
            sequence: 0,
        };
        assert_eq!(state_wrapper.stamp(), 0);
        assert_eq!(state_wrapper.stamp(), 1);
        assert_eq!(state_wrapper.stamp(), 2);
        let mut state_wrapper = PublisherStateWrapper::<()>::with_sequence(10);
        assert_eq!(state_wrapper.stamp(), 10);
        assert_eq!(state_wrapper.stamp(), 11);
    }
}
