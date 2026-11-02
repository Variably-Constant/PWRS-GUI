---
title: "Shared Vec"
weight: 10
---

# SharedVec&lt;T&gt;

![Rust](https://img.shields.io/badge/Rust-1.96+-orange?logo=rust)
![Edition](https://img.shields.io/badge/Edition-2024-blue)
![Layout](https://img.shields.io/badge/Layout-MMF--backed-green)
![Protocol](https://img.shields.io/badge/one_swap_per_slot-lock_free-brightgreen)
![Cross-Process](https://img.shields.io/badge/Cross--Process-yes-success)

Cross-process indexable vector that any number of threads and processes
push to, pop from, read and write at once, none of them waiting on
another. Each slot claims and publishes its element with one
compare-and-swap. Fixed capacity at create. Indexable `get(i)` is
lock-free.

> **The "indexable cross-process vector" primitive.** `get` at
> **5.67 ns**, 1.23x faster than `Mutex<Vec>` (6.99 ns) and 1.28x
> faster than `RwLock<Vec>` (7.23 ns). **`len` 6x faster** (0.77 ns vs
> 4.59 ns): one load and a comparison against a lock cycle. A push
> costs 8.71 ns against an in-process `Mutex<Vec>`'s 5.12 ns.

**Constraints (read first):**

- **Native sidecar integration**: the struct carries a `HandshakeHeader` + `ObservationRing` and implements `subetha_sidecar::AdaptiveInstance`. Wrap in `SidecarBox::new` to register with the global sidecar; raw `create()` / `open()` return the unregistered type unchanged.

- **`T: Copy + 'static`** with `size_of::<T>() <=
  VEC_PAYLOAD_BYTES (56)`; an oversized T returns
  `VecError::PayloadTooLarge` at create/open. (No `Default`
  bound.)
- **Capacity fixed at create**, at most 4,294,967,295 elements:
  `push_back` returns `Full` when exhausted, refused with one read.
- **Writer table fixed at create**: `create(path, capacity, writers)`
  sizes a table of `writers` entries after the slots, one per handle
  that writes the vec at the same time. Each entry holds its handle's
  process id and start time beside how many of its threads write, so a
  writer gives back the count of a process that died. A handle past the
  table writes all the same; if its process dies without dropping it,
  the vec keeps working as though that process still wrote, correct but
  slower, until it is reset. `writer_slots()` returns the size an
  `open` must be given.
- **One swap per slot**: an element of at most 8 bytes at an alignment
  of at most 8 sits in one 64-byte line beside a state word, claimed and
  published by one 16-byte compare-and-swap; a larger one sits in a
  128-byte slot of two buffers and a head naming the current one. A
  reader checks the state or the head and retries only when a write
  landed meanwhile.
- **One writer or several**: with one writer a push claims the slot at
  the length and raises it, and a pop lowers the length alone. A second
  writer's first write moves the vec to the several-writer protocol: a
  push takes its slot by one add to the length, and a pop takes the last
  slot by one compare-and-swap and then lowers the length. The writer
  left moves it back.
- **Holes**: with several writers, a pop whose lowering of the length
  loses to a push leaves a hole below the length. `get` answers `None`
  for it, `set` refuses it with `OutOfBounds`, the next pop steps past
  it, and `for_each` and `snapshot` leave it out.
- **`get(i)` is lock-free**: a read of the length and of the slot,
  `None` at or past the length or at a hole.
- **`len()` is one atomic load**, clamped to the capacity: each slot
  below it holds an element or a hole.
- **`for_each` / `for_each_range` walk without a `Vec`**: each
  slot is read into a local as `get` reads it and handed to the
  closure with its index, holes left out, so a scan costs no
  allocation and no second pass. The range form lets workers take
  disjoint spans.
- **`open_read_only` maps without write access**: `open` needs a
  read+write file handle, which a consumer of a privileged
  producer's vec does not hold. Reads are identical; writes
  return `VecError::ReadOnly`. `is_writable()` reports which. For an
  element of at most 8 bytes it needs a processor that reads 16 bytes
  whole without a compare-and-swap.
- **Needs a 16-byte compare-and-swap**: on a processor without a
  lock-free one, create and open fail with an `Unsupported` I/O error.
- **Full surface**: `push_back` / `pop_back` / `get` /
  `set(i, v)` (writes over the element at `i`) / `clear` /
  `snapshot() -> Vec<T>` / `for_each` / `for_each_range` /
  `capacity` / `writer_slots` / `len` / `is_empty` / `is_writable` /
  `flush` / `flush_async`. `VecError` is `Full` / `OutOfBounds` /
  `LayoutMismatch(Mismatch)` / `InvalidArgument(&'static str)` /
  `PayloadTooLarge` / `ReadOnly` / `IoError(std::io::Error)`.
- **`create` obtains, `reset` truncates**:
  `create(path, capacity, writers)` initializes an empty vec only when
  the path does not yet exist, and otherwise attaches with live
  elements and `len` intact - racing creators all reach the same vec.
  A region built with a different capacity or writer table is a
  `LayoutMismatch`. `reset(path, capacity, writers)` truncates and
  reinitializes, for a caller that owns the path; on Windows it
  succeeds only once every process has unmapped the region. The
  in-place `clear()` empties without truncating, by one swap of the
  vec's top word; a push or a pop racing it lands before it or after
  it.
- **Cross-process backed by MMF.**

---

## Bench evidence

Release build on a Ryzen 7 5700G running Ubuntu in a KVM virtual
machine; each figure is the median of five rounds. `Mutex<Vec<u32>>`
and `RwLock<Vec<u32>>` live in one process; the vec lives in a mapped
file any process can open.

| Op | `SharedVec<u32>` (mmf) | `Mutex<Vec<u32>>` | `RwLock<Vec<u32>>` | mmf relative |
|---|---:|---:|---:|---|
| push_back | 8.71 ns | 5.12 ns | 5.43 ns | 1.70x slower than Mutex |
| get(i) | **5.67 ns** | 6.99 ns | 7.23 ns | **1.23x / 1.28x faster** |
| len() | **0.77 ns** | 4.59 ns | n/a | **6.0x faster** |
| push_back into a full vec | **3.98 ns** | 4.69 ns | n/a | **1.18x faster** |
| push_back from four threads | 46.09 ns | 28.55 ns | n/a | 1.61x slower than Mutex |
| pop_back | 4.71 ns | n/a | n/a | |
| push_back then pop_back | 11.58 ns | n/a | n/a | |

### Reading the trade-offs

1. **A push alone costs more than a `Mutex<Vec>` push.** The vec's
   push claims its slot with a 16-byte compare-and-swap and raises the
   length in a mapped file; the mutex's push takes an uncontended lock
   and appends to memory its own process holds.
2. **get is faster than either lock**: a read of the length and of
   the slot against a full lock cycle.
3. **len is 6x faster**: one load and a comparison against a lock,
   a read and an unlock.
4. **Under four writers both meet on one cache line**: the vec's
   length word and the mutex's lock word. The vec's push also writes
   its own slot's line.
5. **Cross-process visibility** is the architectural lever: neither
   baseline can leave its process.

### Rule 3b bench audit

- **Fair contenders**: `Mutex<Vec<T>>` (textbook) +
  `RwLock<Vec<T>>` (reader-optimized).
- **The clock times the operation alone**: the pop cells push untimed
  before timing their pops, and the four-thread cell's clock runs from
  a barrier's release to the last thread's arrival at the next, with
  the vec emptied between rounds while every thread waits.
- **Sizing**: the push cells clear at a million elements, and the
  controls clear at the same length, so they never reallocate; get and
  len read a vec filled before the clock starts.
- **MMF lifecycle managed**: create + ops + drop + remove_file.

### What the numbers do not show

- **Cross-process append + read**: any process pushes; any
  process reads. The mutex baselines cannot.
- **Concurrent readers**: a get writes nothing, so readers never wait
  on each other or on a writer; the bench times one reader.

---

## Worked examples

### Basic indexable storage

```rust
use subetha_cxc::SharedVec;

let v: SharedVec<u32> = SharedVec::create("/tmp/v.bin", 1024, 1).unwrap();
v.push_back(10).unwrap();
v.push_back(20).unwrap();
assert_eq!(v.get(0), Some(10));
assert_eq!(v.len(), 2);
```

### Cross-process append-only log

```rust
// Writer process:
let v: SharedVec<EventRec> = SharedVec::create("/tmp/log", 1 << 20, 1).unwrap();
for ev in events() { v.push_back(ev).unwrap(); }

// Reader process(es):
let v: SharedVec<EventRec> = SharedVec::open("/tmp/log", 1 << 20, 1).unwrap();
for i in 0..v.len() {
    if let Some(ev) = v.get(i) { process(ev); }
}
```

---

## Use case patterns

### Pattern: cross-process append-only log

Producers push events; observers read by index.

### Pattern: indexable cross-process snapshot

A worker writes a snapshot vector; multiple readers index in.

---

## Known limitations

- **Bounded capacity at create**: no auto-grow.
- **Append + pop_back/clear**: `pop_back` removes the tail and
  `clear` resets len, but there is no per-index removal (a `set`
  overwrites in place).
- **Holes under several writers**: a slot below `len()` can hold no
  element until a pop steps past it.
- **T: Copy** (payload <= 56 bytes): pointer-bearing T need
  indirection.
- **Cross-process backed by MMF.**

---

## Common pitfalls

- **Sizing too small**: `push_back` returns Full once
  capacity exhausts.

- **A writer table smaller than the writing handles**: a handle past
  it writes all the same, but if its process dies without dropping the
  handle, the vec keeps working as though that process still wrote,
  correct but slower, until it is reset.

- **Reading past `len()`**: get(i) returns None, and so does a hole
  below it. Check for None rather than assume every index below
  `len()` holds an element.

- **Wrapping in a Mutex.** Pointless; each slot's swap is already
  concurrency-safe.

- **`snapshot()` to scan.** It allocates a copy the size of the
  vec and leaves a second pass to read it. `for_each` is the
  primitive for looking at every element; `snapshot` is for
  keeping them.

- **`open()` on a file you only have read access to.** It asks
  for a read+write handle and fails. Use `open_read_only`.

---

## References

- Source: `crates/subetha-cxc/src/shared_vec.rs`, with the protocol it
  shares with `RawVec` in `crates/subetha-cxc/src/vec_core.rs`. Unit
  tests cover push, get, set, pop and clear in both slot shapes, holes,
  the capacity bound, cross-handle visibility, the moves between one
  writer and several, the registrations of a process that died, and
  pushers racing one, two and three poppers, and clears, with every
  value a push answered for coming out exactly once.
- Bench: `crates/subetha-cxc/benches/shared_vec.rs` (push, pop, get,
  len, a refused push and a four-thread push, against `Mutex<Vec>` and
  `RwLock<Vec>`).
- Sibling primitive: [Shared Hash Map](../maps/shared-hash-map/) -
  keyed cross-process map.
- Sibling primitive: [Shared Region](../arenas/shared-region/) -
  typed slot allocator with reuse; Vec is the simpler
  append-only variant.
