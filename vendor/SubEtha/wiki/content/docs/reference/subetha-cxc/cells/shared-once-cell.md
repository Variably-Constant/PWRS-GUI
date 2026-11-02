---
title: "Shared Once Cell"
weight: 20
---

# SharedOnceCell&lt;T&gt;

![Rust](https://img.shields.io/badge/Rust-1.96+-orange?logo=rust)
![Edition](https://img.shields.io/badge/Edition-2024-blue)
![Layout](https://img.shields.io/badge/Layout-MMF--backed-green)
![Cross-Process](https://img.shields.io/badge/Cross--Process-yes-success)
![Payload](https://img.shields.io/badge/payload-up_to_56_bytes-informational)

Cross-process write-once cell. A caller that finds it empty copies its
own value into a free slot, the header line's inline slot first for a
value of 32 bytes or less, and publishes it with one swap of the cell's
winner word; the first swap wins, and from then on every caller reads
the winner's slot. No caller waits on another's producer.

> **The cross-process `once_cell::sync::OnceCell` primitive.** When
> multiple processes need to converge on a single agreed-upon value
> (cluster ID, generated key, schema version) and exactly one of
> them generates it, SharedOnceCell is the lock-free answer.

**Constraints (read first):**

- **Native sidecar integration**: the struct carries a `HandshakeHeader` + `ObservationRing` and implements `subetha_sidecar::AdaptiveInstance`. Wrap in `SidecarBox::new` to register with the global sidecar; raw `create()` / `open()` return the unregistered type unchanged.

- **`T: Copy + 'static`**. Payload up to 56 bytes, aligned to at
  most 8.
- **An inline slot, eight slots and a winner word**: a value of 32
  bytes or less is copied into the inline slot in the header line
  while it is free, and any value into one of eight slot lines
  otherwise; the slot is marked filled, and one swap of the winner
  word from zero to that slot publishes it. The winner word is
  written once.
- **One swap elects the value**: of the callers racing to publish,
  the first swap wins and the rest read the value that won. A caller
  waits only while every slot is mid-copy by a live caller.
- **`set(value) -> bool`**: true when `value` is now the cell's
  value, false when another value was published first.
- **`get() -> Option<T>`**: non-blocking; `None` until a value is
  published.
- **`get_or_init(F)`**: a caller that finds the cell empty runs F and
  publishes what it returns; every caller gets the value published
  first. F can run once per caller that finds the cell empty.
- **`get_or_fetch(F) -> Fetch<T>`**: runs F only under the cell's one
  fetch claim, so at most one caller fetches at a time across every
  process, and returns `Fetch::Pending` at once to any other caller. A
  claim whose holder is gone is broken by the next caller.
- **A caller that dies leaves nothing stuck**: a slot whose copier is
  gone, judged by its pid and process start, is freed by the next
  caller that needs one, and a filled slot nobody published is
  published by the next caller that finds every slot taken.
- **`open` rejects mismatched payload size**: same LayoutMismatch
  rejection as SharedCell.
- **`create` obtains, `reset` truncates**: `create(path)` initializes
  an empty cell only when the path does not yet exist, and otherwise
  attaches with a published value intact - racing creators all
  reach the same cell, published once for all of them, so the once
  guarantee holds across them. A region built for a
  different payload type is a `LayoutMismatch`. `reset(path)`
  truncates and reinitializes, for a caller that owns the path; on
  Windows it succeeds only once every process has unmapped the
  region.
- **Cross-process backed by MMF.**

---

## Table of contents

- [What it is](#what-it-is)
- [State machine](#state-machine)
- [Worked examples](#worked-examples)
- [Bench evidence](#bench-evidence)
- [Use case patterns](#use-case-patterns)
- [Known limitations](#known-limitations)
- [Common pitfalls](#common-pitfalls)
- [References](#references)

---

## What it is

`SharedOnceCell<T>` is an MMF-backed write-once cell: a 64-byte
header line holding the inline slot, then eight 64-byte slots, 576
bytes in all. A value of 32 bytes or less published in the inline slot
is read from the header's own line.

```rust
#[repr(C, align(64))]
pub struct OnceHeader {
    pub magic: u32,
    /// Bytes of the value.
    pub size: u32,
    /// Zero until a value is published, then 1 for the inline slot or
    /// 2 + i for slot line i. Written once.
    pub winner: AtomicU64,
    /// The holder word of the caller running a fetch under the cell's
    /// fetch claim, or zero.
    pub fetcher: AtomicU64,
    /// The inline slot's state, as a slot line's.
    pub inline_state: AtomicU64,
    /// The inline slot's value, for a cell of at most
    /// [`ONCE_INLINE_BYTES`].
    pub inline_payload: [u8; ONCE_INLINE_BYTES],
}

#[repr(C, align(64))]
pub struct OnceSlot {
    /// Zero while free, the copier's holder word while it copies, and
    /// [`u64::MAX`] once the value in the slot is whole.
    pub state: AtomicU64,
    pub payload: [u8; ONCE_PAYLOAD_BYTES],
}
```

A slot is taken by a copier, filled, then published by the winner
word; a slot whose copier is gone goes back to free:

```mermaid
graph LR
    F[slot free]
    C[slot copying]
    L[slot filled]
    W[value published]

    F -- "copier swaps in its holder word" --> C
    C -- "copy whole, slot marked filled" --> L
    C -. "copier gone: the next caller frees it" .-> F
    L -- "winner word swapped from zero" --> W

    classDef empty fill:#1e3a5f,stroke:#5b9bd5,color:#e8f1f5
    classDef init fill:#1f4a3a,stroke:#5cb85c,color:#e8f5e8
    classDef done fill:#5a3a1f,stroke:#d5a05b,color:#f5ede0

    class F empty
    class C,L init
    class W done
```

---

## State machine

| Op | No value published | Value published |
|---|---|---|
| `set(v)` | copy v into a free slot and swap the winner word; true when this swap wins | false |
| `get()` | `None` | `Some(value)` |
| `get_or_init(F)` | run F, publish, return the value published first | the value |
| `get_or_fetch(F)` | holding the fetch claim: run F, publish, `Ready` with the value published first; claim held by a live caller: `Pending` | `Ready(value)` |
| `is_initialized()` | false | true |

The only race is on the winner word, and one swap settles it. Copying
into a slot races nobody: each caller copies into a slot it took with
its own swap.

---

## Worked examples

### Single-process get-or-init

```rust
use subetha_cxc::shared_once_cell::SharedOnceCell;

let cell: SharedOnceCell<u64> = SharedOnceCell::create("/tmp/once.bin").unwrap();
let v = cell.get_or_init(|| {
    // Expensive computation; runs at most once.
    42
});
assert_eq!(v, 42);

// Subsequent calls skip the closure.
let v2 = cell.get_or_init(|| panic!("should not run"));
assert_eq!(v2, 42);
```

### Cross-process initialization race

```rust
use subetha_cxc::shared_once_cell::SharedOnceCell;

// Process A and Process B both try to initialize:
let cell_a: SharedOnceCell<u64> = SharedOnceCell::create("/tmp/race.bin").unwrap();
let cell_b: SharedOnceCell<u64> = SharedOnceCell::open("/tmp/race.bin").unwrap();

// Whichever calls set first wins.
let a_won = cell_a.set(100);
let b_won = cell_b.set(200);

// Exactly one returns true.
assert!(a_won ^ b_won);

// Both see the same final value.
assert_eq!(cell_a.get(), cell_b.get());
```

### Non-blocking probe

```rust
use subetha_cxc::shared_once_cell::SharedOnceCell;

let cell: SharedOnceCell<u64> = SharedOnceCell::open("/tmp/once.bin").unwrap();

// Don't block on init; just check.
match cell.get() {
    Some(value) => println!("ready: {value}"),
    None => println!("not yet initialized"),
}
```

---

## Bench evidence

Bench harness: `crates/subetha-cxc/benches/shared_once_cell.rs`.
Measured on Linux in a KVM guest on an AMD Ryzen 7 5700G (16 logical
CPUs), Criterion's default settings, nine runs; each figure is the
median of the runs' medians.

| Op | `std::sync::OnceLock<u64>` | `SharedOnceCell<u64>` |
|---|---:|---:|
| `get()` on a set cell | 5.55 ns | 6.00 ns |
| `get_or_init` on a set cell | 0.66 ns | 1.19 ns |

The `SharedOnceCell` holds a `u64`, so its value sits in the header
line's inline slot and a read touches that one line. Both cells are set
before timing starts, and `get_or_init` is given a closure that panics,
so a run in which either cell called it would stop.

---

## Use case patterns

### Pattern: cross-process schema version

The first process to start sets the schema version; subsequent
processes read it and verify compatibility. No init coordinator
needed.

### Pattern: generated cluster ID

A multi-process cluster needs a unique ID generated once at boot.
SharedOnceCell wins-once semantics elect the generator.

### Pattern: one-shot leader publication

The leader of a leader-election round publishes its identity
(node-id) via SharedOnceCell. Followers read and verify.

### Pattern: expensive initialization sentinel

A computation result (precomputed table, derived config, lookup
index) that any of N processes can generate. SharedOnceCell
ensures it runs once across all of them.

---

## Known limitations

- **56-byte payload limit**: larger values use SHARED_CELL (52
  bytes) or SHARED_VEC. A slot is a 64-byte line: its state word,
  then the 56-byte payload. The inline slot in the header line holds
  up to 32 bytes.
- **A handle never re-initializes**: once a value is published, it
  stays for the region's lifetime; `create` attaches to it. `reset(path)` is the re-initialization path, and on Windows it
  requires every process to have unmapped the region first.
- **`set` returns false on lost race; no payload returned**:
  callers needing the winning value call `get` after `set`.
- **`get_or_init` can run its producer more than once**: every
  caller that finds the cell empty runs it, and one value is
  published. A producer that must run at most once at a time goes
  through `get_or_fetch`.
- **Cross-process backed by MMF.**

---

## Common pitfalls

- **Calling `set` without checking the return value.** The
  caller may have lost the race; their value is discarded.

- **Giving `get_or_init` a producer with side effects.** Each caller
  that finds the cell empty runs it, so a producer that calls a
  service or writes a file does so once per such caller. Use
  `get_or_fetch`, which runs it under one claim and returns
  `Fetch::Pending` to the rest.

- **Treating `Fetch::Pending` as a failure.** It means another caller
  is fetching right now; ask again later, and the value is there once
  that caller publishes.

- **Treating the file as immutable.** It is not; `reset` (or
  deleting the file) resets the state machine.

---

## References

- Source: `crates/subetha-cxc/src/shared_once_cell.rs`, with its unit
  tests.
- Sibling primitive: [Shared Cell](shared-cell/) - the
  multi-write SeqLock cell. SharedCell allows updates;
  SharedOnceCell is write-once.
- Sibling primitive: [Shared Leader Election](../ownership-types/shared-leader-election/) -
  for elections that need lease semantics rather than a single
  one-shot value.
