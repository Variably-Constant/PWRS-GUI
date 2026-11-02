---
title: "Getting started"
weight: 10
---

This tutorial walks from nothing installed to programs that share
SubEtha structures across processes, and to reading what the sidecar
records about them. No prior knowledge of the substrate is assumed.
Every step shows Rust, Python, PowerShell and C side by side, one tab
each, and every program a page shows runs in the project's own tests.
Python, PowerShell and C also have a tutorial of their own,
[SubEtha from Python](../python/),
[SubEtha from PowerShell](../powershell/) and
[SubEtha from C and C++](../c-and-cpp/).

If you already know which structure you want and need a reminder of
which primitive matches your shape, skip to
[role-pair selection](../../how-to/role-pair-selection/).

## The path through

Read the pages in order:

1. [Installation](../installation/). The crates, the Python package, the
   PowerShell module or the C library, and a smoke test; in Rust, the
   substrate microbench too.
2. [Cross-process round trip](../cross-process-roundtrip/). Two
   processes, one file, a shared hash map written by one process and
   read by the other.
3. [Send items between processes](../send-items/). A channel one
   process sends into and another reads later, a reader that waits for
   items as they arrive, and a ring where every sender and reader has
   an id.
4. [Share a counter](../share-a-counter/). Two processes add to one
   atomic counter with no lock, compare-and-exchange hands out a fixed
   number of tickets, a flag publishes a result, and a 32-bit word
   holds bits.
5. [Claim a role across processes](../claim-a-role/). One process holds
   a role and beats to keep it, and once it stops, a standby takes the
   role over.
6. [Move many items per call](../move-many-items/). A run of items, or a
   buffer of them packed end to end, through one call instead of one
   call an item.
7. [Read a failure](../read-a-failure/). What each surface reports when
   a file has another shape or is not there, or an argument is refused,
   and the answers that are not failures.
8. [Reading observations](../reading-observations/). Register a hash
   map with the sidecar, work it, and read what the sidecar counted of
   each kind of operation.

In Rust the channels, `Channel<T>` and `AdaptiveIpc<T>`, answer three
calling conventions on one handle: the sync `send` and `recv`,
`send_blocking` and `recv_blocking`, which park a thread, and
`send_async` and `recv_async`, which suspend a task. The choice is made
per call site, not by a second type. The
[high-level API reference](../../reference/subetha-cxc/high-level-api/)
lays out all three, and [async: cost and scaling](../../how-to/async-paths/)
covers when to reach for each and what the async path measures.

After the tutorial you know enough SubEtha to read the how-to guides for
tasks and the reference pages for the specification. Two pages are worth
reading early: [concurrency and safety](../../explanation/concurrency-and-safety/),
on whether you need locks (you do not), why the rings are thread- and
process-safe, and the one thing you are responsible for; and
[tuning and overrides](../../how-to/tuning-overrides/), for every
environment variable, Cargo feature and build recipe. The
[bridges reference](../../reference/subetha-cxc/bridges/) takes the same
channels across hosts over QUIC, TCP or the Sens-O-Matic reliable-FEC
protocol.
