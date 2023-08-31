//! # Grapevine
//!
//! Grapevine is a lightweight, performant, distributed messaging protocol
//! designed to make writing application logic easy. Think gRPC streams over
//! Multicast.
//!
//! It's important to note that multicast communication is _not possible_
//! over the public internet. Whilst it's possible to use Grapevine across
//! different regions by repeating streams, the immediate use-case of this
//! library is to simplify intra-process communication over a private
//! network.
//!
//! ## Getting Started
//!
//! Being able to reproduce state deterministically through a series of
//! events is a fundamental requirement to synchronizing state across
//! a network.
//!
//! To get started, define your state and the set of actions which can
//! change it. Then implement the `StateSync` trait.
//!
//! ``` no_run
//! use std::{collections::BTreeMap, convert::Infallible};
//! use bytes::{Buf, BufMut};
//! use prost::encoding::{DecodeContext, WireType};
//! use prost::{DecodeError, Message};
//! use grapevine::StateSync;
//!
//! /// A self-contained unit of state.
//! #[derive(Debug)]
//! struct Post {
//!     /// Unique Id.
//!     id: i64,
//!     /// Some data.
//!     text: String,
//!     /// Something which _changes_ through time.
//!     likes: usize,
//! }
//!
//! /// The container for aggregated state. Must be deterministically
//! /// generated given a sequence of events.
//! # #[derive(Default)]
//! struct MyState {
//!     /// A map of unique Ids -> self contained pieces of state.
//!     posts: BTreeMap<i64, Post>,
//! }
//!
//! /// Any event which can update [MyState].
//! # #[derive(Debug)]
//! enum MyStateDelta {
//!     /// A new post was created.
//!     NewPost(Post),
//!     /// An existing post has changed.
//!     NewLikes(i64, usize),
//! }
//!
//! /// You should use protobuf to define this message. See the examples.
//! # #[derive(Debug)]
//! struct MyStateDeltaMessage {
//!     message: MyStateDelta
//! }
//! # impl Message for MyStateDeltaMessage {fn encode_raw<B>(&self, buf: &mut B) where B: BufMut, Self: Sized {
//! #         todo!()
//! #     }
//! #
//! # fn merge_field<B>(&mut self, tag: u32, wire_type: WireType, buf: &mut B, ctx: DecodeContext) -> Result<(), DecodeError> where B: Buf, Self: Sized {
//! #         todo!()
//! #     }
//! #
//! # fn encoded_len(&self) -> usize {
//! #         todo!()
//! #     }
//! #
//! # fn clear(&mut self) {
//! #         todo!()
//! # }
//! # }
//! # impl Default for MyStateDeltaMessage {
//! #    fn default() -> Self {
//! #         todo!()
//! #    }
//! # }
//!
//! // implementing [StateSync] allows you to share this state using Grapevine!
//! impl StateSync for MyState {
//!     type Delta = MyStateDeltaMessage;
//!     type ApplyError = Infallible;
//!
//!     fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError> {
//!         // Describe how Self::Delta deterministically mutates MyState!
//!         match delta.message {
//!             MyStateDelta::NewLikes(id, likes) => {
//!                 let post = self.posts.get_mut(&id).unwrap();
//!                 post.likes += likes;
//!             }
//!             MyStateDelta::NewPost(post) => {
//!                 self.posts.insert(post.id, post);
//!             }
//!         }
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ### Networking
//!
//! There's no single networking solution which will be appropriate for all
//! systems and use cases. A core concept of Grapevine is to offer composable
//! units for both publishing and subscribing to the ongoing changes in
//! a type which is [StateSync], allowing *you* to build a model which
//! fits the scenario being faced most efficiently.
//!
//! ## Crate feature flags
//!
//! - `sqlite`: Persisting both master (publisher) and replica (subscriber)
//! logs of deltas can be handled efficiently by sqlite. Enabled by default.
//! - `mocks`: Library local testing utilities.

pub(crate) mod codec;
pub(crate) mod constants;

mod persistence;
pub mod proto;
pub mod publisher;
mod state_sync;
pub mod subscriber;
pub mod transport;
mod util;

pub use persistence::*;
pub use publisher::{Publisher, PublisherContext};
pub use state_sync::StateSync;
pub use subscriber::MessageSequencer;
pub use util::{destruct_future, should_destruct};
