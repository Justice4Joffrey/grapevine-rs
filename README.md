![fast_grapes](./assets/fast_grapes.png)

# Grapevine

A minimalistic, simple, multicast networking paradigm for distributed systems.

## WARNING:  Not at all complete: work in progres
Examples:

```
# run the subscriber:
cargo run --example order_book_subscriber

# separately, whilst the subscriber is running:
cargo run --example order_book_publisher
```

<!-- TOC -->

* [Grapevine](#grapevine)
    * [Why](#why)
        * [Remote Procedure Calls](#remote-procedure-calls)
        * [Queue Systems (e.g. Kafka)](#queue-systems--eg-kafka-)
    * [What](#what)
        * [Multicast isn't for everyone](#multicast-isnt-for-everyone)
    * [How](#how)
    * [A work in progress](#a-work-in-progress)

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

Not very performant (extra network hops, bottleneck).

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

## A work in progress

Grapevine is by *no means* production ready. Feel free to experiment with it and theorise how it could *one day* be the
answer to all of your messaging woes.