---
title: SubEtha
toc: false
---

<p align="center">
  <img src="images/Logo.png" alt="SubEtha" width="400">
</p>

<p align="center">
  <em>Cross-Context Channel for Rust. Kernel-bypass IPC that spans threads, processes, and disk through a single memory-mapped file.</em>
</p>

> *Don't Panic.*

---

**SubEtha** implements **CXC**, the Cross-Context Channel: a typed channel
that spans every execution context users actually have. Cross-thread within
a process. Cross-process on the same machine. Persisted to disk through
the OS page cache. Cross-machine with a QUIC tunnel at the edge. One byte
layout, one API, **no kernel on the data path** after construction.

The name comes from Douglas Adams's *Hitchhiker's Guide to the Galaxy*.
The Sub-Etha Sens-O-Matic used sub-etheric waves to communicate **around**
conventional channels. CXC does the same thing: it skips the kernel's
"conventional channel" (named pipes, sockets, IPC handles) and writes
directly to user-space memory the kernel page-aliases between participants.

## The Guide

{{< cards >}}
  {{< card link="docs/tutorial/" title="Tutorial" subtitle="Zero to a working cross-process channel in twenty minutes." icon="academic-cap" >}}
  {{< card link="docs/how-to/" title="How-To" subtitle="Targeted recipes: pick a primitive, tune the sidecar, bridge two hosts." icon="book-open" >}}
  {{< card link="docs/reference/" title="Reference" subtitle="Every type, every trait, every guarantee - the dry spec." icon="document-text" >}}
  {{< card link="docs/explanation/" title="Explanation" subtitle="The design rationale: why the byte layout, the dispatcher, the morphs." icon="light-bulb" >}}
{{< /cards >}}

Jumping-off points inside the reference:
[the full primitive catalog](docs/reference/subetha-cxc/catalog/) ·
[rings, stacks, and queues](docs/reference/subetha-cxc/rings/) ·
[the adaptive shape-morphing ring](docs/reference/subetha-cxc/rings/shared-ring-adaptive/) ·
[cross-platform benchmarks](docs/reference/subetha-cxc/cross-platform-benchmarks/) ·
[the crate family](docs/reference/).

## How fast?

Measured on six hosts (native Windows and WSL2 on a Ryzen 7 2700; an
Ubuntu VM and a FreeBSD VM on a Ryzen 7 5700G; a 2012 Intel Mac; a
3-vCPU EPYC 7552 VPS), 10,000 round-trips, 8-byte payloads,
cross-process between parent and child. A marker appears only on the
hosts a contender could run on:

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/platform_ipc_dotplot_dark.png">
  <img alt="Cross-process IPC one-way latency, every contender on every host" src="images/platform_ipc_dotplot.png">
</picture>

Across all six hosts the four pinned SubEtha shapes land at 36.8-114.9
ns one-way. Each host's fastest shape is 126-499x faster than that
host's fastest kernel IPC, and 4.9-11.5x faster than iceoryx2's
zero-copy shared memory where it builds. The exact numbers on the
Ubuntu VM:

| Method | One-way latency | vs SubEtha |
|---|---:|---:|
| **SubEtha pinned rings (MMF), 4 shapes** | **50.1-80.2 ns** | **1.00x** |
| iceoryx2 (Eclipse zero-copy) | 258 ns | **5.2x slower** |
| Named pipe (`interprocess`) | 16,591 ns | **331x slower** |
| Anonymous stdio pipe | 17,219 ns | **343x slower** |
| `ipc-channel` (Mozilla) | 20,157 ns | **402x slower** |
| UDP loopback | 21,042 ns | **419x slower** |
| TCP loopback | 24,813 ns | **495x slower** |
| ZeroMQ (`ipc://` REQ/REP)\* | 70,891 ns | **1315x slower** |

\* ZeroMQ is from an earlier Linux run (optional `zmq-bench` feature,
links `libzmq`), compared against that run's fastest SubEtha shape at
53.9 ns.

The methodology audit and reproducibility instructions are in
[`docs/CROSS_PROCESS_IPC_PERFORMANCE.md`](https://github.com/Variably-Constant/SubEtha/blob/main/docs/CROSS_PROCESS_IPC_PERFORMANCE.md).

### In-process, too

The same rings run thread-to-thread with no memory-mapped file, against
`crossbeam_channel`, `flume`, `rtrb`, and `std::sync::mpsc` (1,000,000
items, 16-byte payload, median-of-11), benched on every host:

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/platform_inprocess_dotplot_dark.png">
  <img alt="In-process throughput by host: SubEtha ring shapes vs crossbeam, flume, rtrb, std::sync::mpsc" src="images/platform_inprocess_dotplot.png">
</picture>

At 4 producers / 4 consumers a SubEtha shape leads on Windows, WSL2,
Linux and FreeBSD: the composed MPMC grid on Linux (36.6 ns/item, 2.8x
faster than `crossbeam_channel`), the single Vyukov ring on the other
three. `crossbeam_channel` takes that cell on the 2012 Intel Mac, and
`flume` edges the grid on the 3-vCPU EPYC VPS (362.0 vs 375.2 ns/item).
rtrb's purpose-built SPSC leads raw 1P/1C everywhere. Each contender
appears only in the scenarios its API supports - rtrb (SPSC) in 1P/1C
alone, `std::sync::mpsc` (single-consumer) through 4P/1C, crossbeam and
flume (MPMC) in all three - and none of them crosses a process
boundary: this whole field is in-process only. SubEtha is the same
structure morphing across all four shapes, running cross-thread,
cross-process, and cross-host.

## What it looks like

{{< tabs >}}

{{< tab name="Rust" >}}
```rust
use subetha_cxc::AutoIpc;

let chan = AutoIpc::new("/tmp/events.bin")
    .capacity(64)
    .build_channel::<u64>()?;

chan.send(&42)?;                  // non-blocking
let v = chan.recv()?;
assert_eq!(v, 42);

chan.send_blocking(&42, None)?;   // parks the thread
chan.send_async(&42).await?;      // suspends the task, any executor
```
{{< /tab >}}

{{< tab name="Python" >}}
```python
import os
import tempfile

import subetha

chan = subetha.Channel(os.path.join(tempfile.gettempdir(), "events.bin"), capacity=64)

chan.send(b"42")                  # False when full, never waits
assert chan.recv() == b"42"       # None when empty

chan.send_for(b"42", timeout=1)   # waits up to a second for room
assert chan.recv_for(timeout=1) == b"42"
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
Import-Module SubEtha
$chan = New-SubEthaChannel -Path (Join-Path ([IO.Path]::GetTempPath()) 'events.bin') -Capacity 64

$null = $chan.Send('42')                            # False when full, never waits
[Text.Encoding]::UTF8.GetString($chan.Recv())       # 42; Recv answers $null when empty

$null = $chan.SendFor('42', 1)                      # waits up to a second for room
[Text.Encoding]::UTF8.GetString($chan.RecvFor(1))   # 42
```
{{< /tab >}}

{{< /tabs >}}

One handle answers all three call styles. Sync, blocking, and async are
not separate types; you pick per call. The async future is a plain
`std::future::Future`, so it runs on tokio or on the crate's own
runtime-free `block_on`.

Same file path, opened from another thread or another process or after
a reboot: same behavior. The `AutoIpc` builder infers the workload from
declarative hints: no hints, or more producers and consumers, routes to
`SharedRing` streaming; `.batch_size(64)` routes to work-stealing, where
the dispatcher picks KHL for one consumer and URD per-thief mailboxes
for several.

## The crate family

The principal user-facing crate is
[`subetha-cxc`](docs/reference/subetha-cxc/_index.md), the CXC
implementation. `Channel<T>`, `AdaptiveIpc<T>`, `AutoIpc`, the MMF
dispatcher, and more than sixty MMF-backed primitives: `SharedRing`,
the `SharedDeque` family (Chase-Lev, plus the SubEtha-native KHL / KHPD / LOH /
URD variants), `SharedHashMap`, `SharedRWLock`, `SharedSemaphore`,
`SharedLRUCache`, `SharedBTreeMap`, `OwnerLease`, `HeartbeatTable`,
`EpochBarrier`. Same MMF, three deployment modes. Map the file from a
second thread and it works cross-thread. Map it from a second process
and it works cross-process. Let the kernel flush dirty pages to disk
and it persists. **There is no separate "shared memory" abstraction
versus a "disk" abstraction. The MMF is both at once.**

The companion crate is
[`subetha-pointers`](docs/reference/subetha-pointers/_index.md): exotic
pointer types in eight families, each pairing an address with extra
bits the consumer can act on without going through the data.

- `UmbraPointer<T>` carries a 4-byte content prefix beside the pointer
  for short-circuit equality before deref.
- `BloomPointer<T>` carries a 64-bit Bloom filter for probabilistic
  set membership without touching the target.
- `CardinalityPointer<T>` carries a log2 cardinality estimate for
  size-class branching.
- `KStepPointer<T>` encodes a log2 stride for strided iteration.
- `KTower2<T>` and `KTower3<T>` encode multi-segment
  zone-region-offset addressing.
- `SelfDescPointer<T>` carries a type ID and a layout shape for
  dispatch over a heterogeneous container.
- `VersionedPointer<T>`, `HlcVersionedPointer<T>` and
  `VectorClockPointer<T, N>` carry a version counter, a hybrid logical
  clock or a vector clock, for MVCC and causal ordering.
- `ReadableCapability<'a, T>` and `WritableCapability<'a, T>` carry
  bounds and permissions checked in software at every access
  (CHERI-style).

The substrate is [`subetha-core`](docs/reference/subetha-core/_index.md)
(handshake header, observation ring, marshal trait, axis-signature
catalog, CPUID helpers). The control plane is
[`subetha-sidecar`](docs/reference/subetha-sidecar/_index.md)
(per-NUMA scan thread, policy, `SidecarBox`, `AdaptiveInstance`).
`AdaptiveIpc::send` carries a `TypeId`-monomorphized branch that LLVM
resolves to a constant at codegen time, routing `T = u64` to an 8-byte
buffer with no opt-in; its A/B bench measures the two paths within
noise, since LLVM already inlines the generic `u64` path to equivalent
code. **All four crates build on stable Rust 1.96+.**

## Three doors into the Guide

If you came to write code, start with the
[getting-started tutorial](docs/tutorial/getting-started/). Twenty
minutes from zero to your first working cross-process channel.

If you came to understand the architecture before touching code,
start with the [architecture overview](docs/explanation/architecture/)
and read it alongside the
[frozen-handshake explanation](docs/explanation/frozen-handshake/) for
the *why*.

If you already know your concurrency shape, the
[role-pair selection guide](docs/how-to/role-pair-selection/) maps
shapes to primitives without ceremony.

## Reading order

The four sections of this guide follow
[Diataxis](https://diataxis.fr/): Tutorial, How-To, Reference,
Explanation. They are independent. Read in any order.

- **[Tutorial](docs/tutorial/)** is a teacher. It assumes nothing
  and walks you from zero to a working channel.
- **[How-To](docs/how-to/)** is a recipe book. Each page solves one
  specific problem in five minutes.
- **[Reference](docs/reference/)** is the operating manual. Every
  type, every parameter, every guarantee. Look things up.
- **[Explanation](docs/explanation/)** is the background. How the
  byte layout works, what the dispatcher does, and how one file serves
  three deployment modes. Read when you want to understand, not when
  you want to ship.

## License

SubEtha is licensed under the MIT License. See
[LICENSE-MIT](https://github.com/Variably-Constant/SubEtha/blob/main/LICENSE-MIT)
for the full text.

> *"So long, and thanks for all the pipes."*
