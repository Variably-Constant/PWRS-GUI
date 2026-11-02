---
title: Tutorial
linkTitle: Tutorial
weight: 1
sidebar:
  open: true
---

If you are new to SubEtha, start here. The tutorial takes you from
zero to your first cross-process channel in roughly twenty minutes,
then walks the observation pipeline so you know what the sidecar
is recording about your traffic.

Read the pages in order. Each shows its steps in Rust, Python,
PowerShell and C side by side:

1. [Getting started](getting-started/) - the path through the chapters below.
2. [Installation](installation/) - the crates, the Python package, the PowerShell module or the C library, and a smoke test.
3. [Cross-process round-trip in 30 lines](cross-process-roundtrip/) - one process writes a shared hash map, another reads it.
4. [Send items between processes](send-items/) - a channel one process sends into and another reads later, a reader that waits for items as they arrive, and a ring where every sender and reader has an id.
5. [Share a counter](share-a-counter/) - two processes add to one atomic counter with no lock, compare-and-exchange hands out a fixed number of tickets, a flag publishes a result, and a 32-bit word holds bits.
6. [Claim a role across processes](claim-a-role/) - one process holds a role and beats to keep it, and once it stops, a standby takes the role over.
7. [Move many items per call](move-many-items/) - a run of items, or a buffer of them packed end to end, through one call instead of one call an item.
8. [Read a failure](read-a-failure/) - what each surface reports when a file has another shape or is not there, or an argument is refused, and the answers that are not failures.
9. [Reading observations](reading-observations/) - register a hash map with the sidecar, work it, and read what the sidecar counted of each kind of operation.

Arriving with a C or C++ toolchain rather than a Rust one? Read
[SubEtha from C and C++](c-and-cpp/) instead of the pages above: it
installs the library, links a program against it, sends a message
through a ring, and reads a failure, without asking you to write Rust.

Arriving from Python? The pages above carry Python tabs, and
[SubEtha from Python](python/) goes further through the binding: two
processes sharing a counter,
a message crossing between them, a role one of them claims, and the two
call shapes that cost far less than one call an item.

Arriving from PowerShell? The pages above carry PowerShell tabs, and
[SubEtha from PowerShell](powershell/) goes further through the
`SubEtha` module: a counter shared by
two sessions, a message through a ring and through the pipeline, a
role one session claims, and the packed shapes that cost less than a
call an item.
