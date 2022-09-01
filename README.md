![fast_grapes](./assets/fast_grapes.png)

# Grapevine

A minimalistic, simple, multicast networking paradigm for distributed systems.

<!-- TOC -->

* [Grapevine](#grapevine)
    * [Why](#why)
        * [Remote Procedure Calls](#remote-procedure-calls)
        * [Queue Systems (e.g. Kafka)](#queue-systems--eg-kafka-)
    * [What](#what)
        * [Multicast isn't for everyone](#multicast-isnt-for-everyone)
    * [How](#how)
    * [A work in progress](#a-work-in-progress)
        * [But I want to talk both ways over multicast](#but-i-want-to-talk-both-ways-over-multicast)
        * [Caution](#caution)
    * [TODO:](#todo-)

<!-- TOC -->

## Why

Tracking state changes across distributed processes is difficult!

Generally speaking, we want to build event-driven solutions to distributed problems. If we can build each part of our
system as a

A couple of popular alternative solutions:

### Remote Procedure Calls

Generally stateful/connection oriented.
Must handle errors at call site

### Queue Systems (e.g. Kafka)

## What

By leveraging the power of [UDP multicast](https://en.wikipedia.org/wiki/Multicast), services sharing messages can be
decoupled without _any_ network overhead. There isn't a faster, or more streamlined, way to communicate between services
than with direct UDP messages.

### Multicast isn't for everyone

Be aware: you cannot use multicast to communicate over the public internet. Inter-network communication must be handled
by
stream repeaters.

Grapevine puts the power in *your hands* to solve the problem *you face* with a collection of low-level utilities which
can be composed to create a high-level solution.

## How

Publishers send updates as diffs/deltas over UDP (fire and forget) on multicast. Subscribers listen to a multicast
address, applying each patch to their local version of the data.

Publishers _also_ cycle through the 'leaves' (some small, self-contained unit) of the source-of-truth data, and
send a snapshot of the data over the network.

To synchronise on startup, the subscriber will listen to snapshots until its received an entry twice. This means it
has listened to every snapshot message at least once.

To handle packet loss/dropped messages, the subscriber will ensure it sees every sequence number (not necessarily in
order). In the case where a message is missed, the subscriber will listen to snapshots again until it's back in sync.

The core feature of this is the StateStorage trait.

``` rust
struct Post {
  id: i64
  text: String,
  likes: usize
}
  
struct MyState {
  posts: BtreeMap<i64, Post>
}

enum MyStateDelta {
  NewPost(Post),
  NewLikes(usize)
}

impl StateSync for MyState {
  type Delta = MyStateDelta;
  // coming soon!
  type Snapshot = Post;
  type ApplyError = Infallible;
  
  fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError> {
    // Describe how MyStateDelta deterministically mutates MyState!
    todo!()
  }
  
  // coming soon!
  fn apply_snapshot(&mut self, snapshot: Self::Snapshot) -> Result<(), Self::ApplyError> {
    self.likes.insert(snapshot.id, snapshot);
    Ok(())
  }
} 
```

### The pieces to the puzzle

####

## A work in progress

Grapevine is by *no means* production ready. Feel free to experiment with it and theorise how it could *one day* be the
answer to all of your messaging woes.

Contributions are more than welcome.

#### But I want to talk both ways over multicast

No, you don't. One-way dataflow is much cleaner to implement and easier to reason with. For 2 way-communication, create
Publishers on either side and communicate on a new multicast address.

#### Caution

This isn't suitable for an event-based paradigm where it's imperative to store every _event_, such as CQRS. The
trade-off being made is sacrificing perfect visibility of each event for simplicity, whilst keeping the guarantee of
eventual consistency.

## TODO:

- publisher recv 'send snapshot', otherwise send check-ins
- how do we work in 'delete this key' style messages -> (simple?)
- receiver should be unhappy when it times out request
- snapshots must currently be under 4096 bytes, as there's no recovery
    * think of the best solution (multi message snaps?)
    * what actually happens when we send a UDP > packet size?
- fuzz testing, but we need to make sure that fuzz actions are valid
    * increasing timestamps
    * increasing sequence IDs
    * cancel requests point to actual sequence/order IDs