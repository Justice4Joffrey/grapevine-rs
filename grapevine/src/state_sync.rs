use std::error::Error;

use prost::Message;

/// Implement this trait to facilitate sharing
pub trait StateSync: Default + Send + Sync + 'static {
    /// A unit of change to the internal state.
    type Delta: Message + Default;
    /// Failure mode applying an update to the internal state.
    type ApplyError: Error;

    fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError>;
    fn reset(&mut self) {
        *self = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::proto::tree_state::*;

    #[test]
    fn state_sync() {
        use leaf_delta::Delta;

        let mut state = TreeState::default();
        let s1 = Leaf { id: 1, a: 2, b: 3 };
        let s2 = Leaf { id: 2, a: 4, b: 6 };
        let s3 = Leaf { id: 3, a: 6, b: 9 };
        let s4 = Leaf { id: 4, a: 8, b: 12 };

        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::NewLeaf(NewLeaf { leaf: s1.clone() })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::NewLeaf(NewLeaf { leaf: s2.clone() })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::NewLeaf(NewLeaf { leaf: s3.clone() })),
            })
            .unwrap();
        assert_eq!(
            state,
            TreeState {
                leaves: BTreeMap::from([
                    (s1.id, s1.clone()),
                    (s2.id, s2.clone()),
                    (s3.id, s3.clone())
                ])
            }
        );
        // Apply some randomish deltas
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::EditA(EditA {
                    leaf: EditLeaf {
                        id: s1.id,
                        value: 10,
                    },
                })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::EditB(EditB {
                    leaf: EditLeaf {
                        id: s1.id,
                        value: -10,
                    },
                })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::EditB(EditB {
                    leaf: EditLeaf {
                        id: s3.id,
                        value: 100,
                    },
                })),
            })
            .unwrap();
        assert_eq!(state.leaves.get(&s2.id).unwrap(), &s2);
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::DeleteLeaf(DeleteLeaf { id: s2.id })),
            })
            .unwrap();

        assert!(state.leaves.get(&s2.id).is_none());
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::NewLeaf(NewLeaf { leaf: s4.clone() })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::EditA(EditA {
                    leaf: EditLeaf {
                        id: s1.id,
                        value: 20,
                    },
                })),
            })
            .unwrap();
        state
            .apply_delta(LeafDelta {
                delta: Some(Delta::EditB(EditB {
                    leaf: EditLeaf {
                        id: s4.id,
                        value: -100,
                    },
                })),
            })
            .unwrap();
        assert_eq!(
            state,
            TreeState {
                leaves: BTreeMap::from([
                    (
                        s1.id,
                        Leaf {
                            id: s1.id,
                            a: 32,
                            b: -7
                        }
                    ),
                    (
                        s3.id,
                        Leaf {
                            id: s3.id,
                            a: 6,
                            b: 109
                        }
                    ),
                    (
                        s4.id,
                        Leaf {
                            id: s4.id,
                            a: 8,
                            b: -88
                        }
                    )
                ])
            }
        );

        state.reset();
        assert_eq!(state, TreeState::default());
    }
}
