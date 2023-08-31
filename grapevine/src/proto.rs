pub mod grapevine {
    use std::cmp::Ordering;

    include!("./generated/grapevine.rs");

    impl Eq for RawMessage {}

    impl PartialOrd for RawMessage {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for RawMessage {
        fn cmp(&self, other: &Self) -> Ordering {
            self.metadata.sequence.cmp(&other.metadata.sequence)
        }
    }
}

pub mod recovery {
    include!("./generated/recovery.rs");
}

#[cfg(feature = "mocks")]
pub mod tree_state {
    use std::{collections::BTreeMap, convert::Infallible};

    use crate::{proto::tree_state::leaf_delta::Delta, StateSync};
    include!("./generated/tree_state.rs");

    /// A simple state sync that just keeps a map of leaf nodes.
    #[derive(Default, Debug, PartialEq, Clone)]
    pub struct TreeState {
        pub leaves: BTreeMap<i32, Leaf>,
    }

    impl StateSync for TreeState {
        type ApplyError = Infallible;
        type Delta = LeafDelta;

        fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError> {
            match delta.delta.unwrap() {
                Delta::EditA(edit) => {
                    let x = self.leaves.get_mut(&edit.leaf.id).unwrap();
                    x.a += edit.leaf.value;
                }
                Delta::EditB(edit) => {
                    let x = self.leaves.get_mut(&edit.leaf.id).unwrap();
                    x.b += edit.leaf.value;
                }
                Delta::NewLeaf(leaf) => {
                    self.leaves.insert(leaf.leaf.id, leaf.leaf);
                }
                Delta::DeleteLeaf(leaf) => {
                    self.leaves.remove(&leaf.id);
                }
            }
            Ok(())
        }
    }
}
