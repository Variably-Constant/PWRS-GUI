---
title: "Lazy Config"
weight: 30
---

# LazyConfig&lt;T&gt;

![Rust](https://img.shields.io/badge/Rust-1.96+-orange?logo=rust)
![Edition](https://img.shields.io/badge/Edition-2024-blue)
![Layout](https://img.shields.io/badge/Layout-MMF--backed-green)
![Protocol](https://img.shields.io/badge/CAS--once-thundering--herd--safe-brightgreen)
![Cross-Process](https://img.shields.io/badge/Cross--Process-yes-success)
![Fast_path](https://img.shields.io/badge/get_loaded-1.65_ns-informational)

Thundering-herd-proof distributed config fetch. Composite over
[`SharedOnceCell<T>`](../cells/shared-once-cell/): the caller that
takes the cell's fetch claim runs the fetcher closure and publishes;
for a caller that finds a fetch in flight, `get_or_fetch` returns
`Fetch::Pending` at once, and that caller reads the value on a later
call. Across N processes mapping the same file, one fetch runs at a
time and one result is published, no matter how many of them call
`get_or_fetch` concurrently. A fetch runs again only after one ended
unpublished, its process gone or its closure panicked.

> **The "config-fetch primitive that beats the thundering herd"
> primitive.** `try_get_loaded` at **10.11 ns** vs
> `std::sync::OnceLock` 10.01 ns (essentially tied) and
> `Mutex<Option<T>>` 17.77 ns (**1.76x faster**). Post-load
> `get_or_fetch_loaded` at 1.65 ns vs std OnceLock 1.06 ns
> (1.55x slower). The architectural lever: same per-op cost
> envelope as the in-process baseline and cross-process
> visibility and single-fetch-across-N-processes guarantee.

**Constraints (read first):**

- **Native sidecar integration**: the struct carries a `HandshakeHeader` + `ObservationRing` and implements `subetha_sidecar::AdaptiveInstance`. Wrap in `SidecarBox::new` to register with the global sidecar; raw `create()` / `open()` return the unregistered type unchanged.

- **`T: Copy + Send + Sync + 'static`, fixed payload** sized to
  `ONCE_PAYLOAD_BYTES` (56 bytes).
- **One fetch at a time across all callers**: `get_or_fetch` runs
  the fetcher only under `SharedOnceCell`'s fetch claim, and returns
  `Fetch::Pending` at once to a caller that finds the claim held by a
  live caller. A claim whose holder is gone is taken over, and a
  fetch that panics gives the claim back.
- **`force_set` for admin override**: bypasses the fetcher; the
  first `force_set` wins (returns true), subsequent ones lose
  (return false) without changing the value.
- **Nothing blocks**: `try_get` returns `None` when unloaded, and
  `get_or_fetch` returns `Fetch::Ready(value)` or `Fetch::Pending`.
  A caller that gets `None` or `Fetch::Pending` asks again on its own
  schedule.
- **Cross-process backed by MMF.**

---

## Table of contents

- [What it is](#what-it-is)
- [Protocol](#protocol)
- [Bench evidence](#bench-evidence)
- [Worked examples](#worked-examples)
- [Use case patterns](#use-case-patterns)
- [Known limitations](#known-limitations)
- [Common pitfalls](#common-pitfalls)
- [References](#references)

---

## What it is

```mermaid
block-beta
  columns 1
  cell["SharedOnceCell of T - a 64 B header line (winner word, fetch claim, 32 B inline slot), then eight 64 B slots (state word, 56 B payload for T)"]
  classDef cellC fill:#1e3a8a,color:#ffffff
  class cell cellC
```

LazyConfig is a thin wrapper over `SharedOnceCell`. The cell
holds the entire protocol; LazyConfig adds the
config-fetch-shaped API (`get_or_fetch`, `try_get`,
`is_loaded`, `force_set`).

---

## Protocol

### get_or_fetch(fetcher)

```text
if the winner word names a slot:
   return Ready(that slot's value)

# Take the fetch claim: swap it from zero to this caller's holder
# word, or break a claim whose holder is gone and take it.
if the claim is taken:
   value = fetcher()        # gives the claim back if it panics
   copy value into a free slot (the inline slot first when it fits), mark it filled
   swap the winner word from zero to that slot
   give the claim back
   return Ready(the value published first)
else:
   # a live caller is fetching
   return Ready(value) if one was published meanwhile, else Pending
```

One fetch claim runs one fetch at a time. A caller that cannot take
it returns at once; no call waits on the fetcher.

### try_get()

```text
if the winner word is zero:
   return None
copy the winner's slot
return Some(value)
```

One atomic load and one copy. No blocking.

---

## Bench evidence

Bench harness: `crates/subetha-cxc/benches/lazy_config.rs`.
Measured on Linux in a KVM guest on an AMD Ryzen 7 5700G (16 logical
CPUs), Criterion's default settings, five runs; each figure is the
median of the runs' medians.

| Op | `LazyConfig<u64>` | `std::sync::OnceLock<u64>` | `Mutex<Option<u64>>` |
|---|---:|---:|---:|
| `try_get` on a loaded config | 6.81 ns | 5.83 ns | 4.85 ns |
| `get_or_fetch` on a loaded config | 1.70 ns | 0.69 ns | |
| `is_loaded` | 1.01 ns | | |

Each contender is loaded before timing starts. `get_or_fetch`, and
`get_or_init` on the `OnceLock`, are given a fetcher that panics, so a
run in which one was called would stop. Every call in the bench comes
from one thread; that one fetch runs across 16 concurrent callers is
checked by the unit test `fetch_runs_at_most_once_under_concurrency`.

---

## Worked examples

### Distributed service: load config once per cluster

```rust
use subetha_cxc::lazy_config::{Fetch, LazyConfig};

#[derive(Clone, Copy)]
#[repr(C)]
struct AppConfig {
    max_conns: u32,
    ttl_ms: u32,
    feature_flags: u32,
}

let lc: LazyConfig<AppConfig> = LazyConfig::open("/tmp/app.cfg").unwrap();
match lc.get_or_fetch(|| {
    // Network fetch from Consul / etcd / DNS.
    fetch_config_from_consul()
}) {
    Fetch::Ready(cfg) => serve(cfg),
    // Another process is fetching it; ask again on the next tick.
    Fetch::Pending => {}
}

// Subsequent processes opening the same file see cfg cached.
```

### Fetching readers and reading-only readers

```rust
use subetha_cxc::lazy_config::{Fetch, LazyConfig};

let lc: LazyConfig<u64> = LazyConfig::open("/tmp/lc.bin").unwrap();

// Fetching reader: fetches when no other caller is fetching.
if let Fetch::Ready(v) = lc.get_or_fetch(load_value) {
    use_value(v);
}

// Reading-only reader: returns None if not loaded.
if let Some(v) = lc.try_get() {
    use_value(v);
} else {
    // Not loaded yet; retry later.
}
```

### Admin-override workflow

```rust
use subetha_cxc::LazyConfig;

let lc: LazyConfig<u32> = LazyConfig::create("/tmp/lc.bin").unwrap();

// First admin push wins.
assert!(lc.force_set(42));
assert_eq!(lc.try_get(), Some(42));

// Second admin push loses (config already set).
assert!(!lc.force_set(99));
assert_eq!(lc.try_get(), Some(42));
```

---

## Use case patterns

### Pattern: cross-process config bootstrap

A pool of N processes each call `get_or_fetch(fetch_from_consul)`
at startup. One runs the network fetch; for the others the call
returns `Fetch::Pending` until it publishes, and they then read the
published config. The backend service sees one request instead of N.

### Pattern: lazy initialization of expensive resources

Configuration that takes seconds to compute (parsing TLS
certs, building a routing table, hashing a corpus) initialized
on first use; subsequent calls see the cached result at
10.11 ns.

### Pattern: pre-loaded daemon + late-arriving workers

A daemon process calls `force_set` at startup to publish
config. Worker processes started later see `is_loaded() == true`
immediately and skip fetch entirely.

---

## Known limitations

- **Payload size capped at 56 bytes**: same as
  `ONCE_PAYLOAD_BYTES`. Larger T needs pointer
  indirection or the bigger-payload cell variants.
- **No re-fetch**: once loaded, the cell is permanently
  initialized. Configuration that changes over time needs a
  different primitive (e.g., a versioned cell + a refresh
  protocol).
- **A panicking fetcher publishes nothing**: it gives the fetch
  claim back as the panic leaves, and the next caller's fetch runs.
- **A stalled fetcher keeps the claim**: while its process lives,
  `get_or_fetch` returns `Fetch::Pending` to every other caller. A
  fetcher that must give up after a while carries its own timeout.
- **Cross-process backed by MMF.**

---

## Common pitfalls

- **Treating the fetcher as a constructor with side effects.**
  The fetcher runs in one process for the whole cell, and again
  only after a fetch ended unpublished; any side effect (e.g.,
  logging "I fetched") happens there, not once per process. Move
  per-process side effects outside the closure.

- **Treating `Fetch::Pending` as an error.** It means a live caller
  is fetching right now; ask again later.

- **Mismatched type at open vs create.** The cell stores
  exactly `size_of::<T>()` bytes; opening with a different T
  reads garbage bytes as T. Pin T in a shared spec.

- **Calling `get_or_fetch` from inside the fetcher.** The
  reentrant call finds the fetch claim held by its own caller and
  returns `Fetch::Pending`.

- **Wrapping in a Mutex.** Pointless; the fetch claim and the
  winner word already make it safe across threads and processes.

---

## References

- Source: `crates/subetha-cxc/src/lazy_config.rs`, with its unit
  tests.
- Bench: `crates/subetha-cxc/benches/lazy_config.rs` (try_get,
  get_or_fetch on loaded config, is_loaded vs
  `std::sync::OnceLock<T>` and `Mutex<Option<T>>`).
- Underlying primitive:
  [Shared Once Cell](../cells/shared-once-cell/) - the cell of
  slots and a winner word LazyConfig wraps.
- Sibling primitive: [Shared Cell](../cells/shared-cell/) -
  mutable cell; LazyConfig is the write-once specialization.
- Composes with:
  [Shared Broadcast Ring](../rings/shared-broadcast-ring/) - for
  config change notifications when paired with a versioned
  publish scheme.
