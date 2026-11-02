---
title: "Send items between processes"
weight: 31
---

A channel is a queue that lives in a file. One process puts items in,
another takes them out in the order they went in, and an item waits in
the file until someone takes it, so the reader can come along later.
This page sends items to a reader that runs afterwards, has a reader
wait for items as they arrive, and then does the same through a ring,
where every sender and reader has an id of its own. Save the programs
for your surface in one folder and run them from it in order.

## Send now, read later

The sender makes a channel with room for 16 items, sends three, and
exits. Making a channel starts its file afresh, so the sender is the
program that runs first.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/send.rs"}
use subetha::cxc::{Channel, MmfWorkloadShape, SlotValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Room for 16 items, for one sender and one reader.
    let shape = MmfWorkloadShape::StreamingMpmc { n_producers: 1, n_consumers: 1 };
    let channel = Channel::<SlotValue>::create("items.chan", shape, 16)?;
    for word in ["first", "second", "third"] {
        // An item is up to 52 bytes, and it keeps its length.
        channel.send(&SlotValue::from_bytes(word.as_bytes())?)?;
    }
    println!("sent 3 items");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="send.py"}
import subetha

# Room for 16 items, for one sender and one reader.
channel = subetha.Channel("items.chan", capacity=16)
for word in [b"first", b"second", b"third"]:
    # send answers False when the channel is full.
    if not channel.send(word):
        raise RuntimeError("the channel is full")
print("sent 3 items")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="send.ps1"}
Import-Module SubEtha

# Room for 16 items, for one sender and one reader.
$channel = New-SubEthaChannel -Path items.chan -Capacity 16
foreach ($word in 'first', 'second', 'third') {
    # Send answers False when the channel is full.
    if (-not $channel.Send($word)) { throw 'the channel is full' }
}
$channel.Dispose()
'sent 3 items'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="send.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* Room for 16 items, for one sender and one reader. */
    subetha_handle channel;
    if ((rc = subetha_channel_create("items.chan", 16, 1, 1, SUBETHA_MODE_DEFAULT, &channel)) != SUBETHA_OK)
        return fail("create", rc);
    const char *words[] = {"first", "second", "third"};
    for (int i = 0; i < 3; i++) {
        /* An item is up to SUBETHA_CHANNEL_ITEM_MAX bytes, and it keeps its
           length. */
        rc = subetha_channel_try_send(channel, (const uint8_t *)words[i], strlen(words[i]));
        if (rc != SUBETHA_OK) return fail("send", rc);
    }
    if ((rc = subetha_handle_destroy(channel)) != SUBETHA_OK) return fail("destroy", rc);
    printf("sent 3 items\n");
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
sent 3 items
```

The reader joins the channel with `open`, naming the capacity it was
made with, and takes items until there are none left.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/receive.rs"}
use subetha::cxc::{ApiError, Channel, SlotValue, TransportError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Channel::<SlotValue>::open("items.chan", 16)?;
    loop {
        // recv answers Empty once every item has been taken.
        match channel.recv() {
            Ok(item) => println!("received {}", String::from_utf8_lossy(item.as_bytes())),
            Err(ApiError::Transport(TransportError::Empty)) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="receive.py"}
import subetha

channel = subetha.Channel.open("items.chan", capacity=16)
# recv answers None once every item has been taken.
while (item := channel.recv()) is not None:
    print("received", item.decode())
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="receive.ps1"}
Import-Module SubEtha

$channel = Open-SubEthaChannel -Path items.chan -Capacity 16
# Recv answers $null once every item has been taken.
while ($null -ne ($item = $channel.Recv())) {
    "received $([Text.Encoding]::UTF8.GetString($item))"
}
$channel.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="receive.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle channel;
    if ((rc = subetha_channel_open("items.chan", 16, SUBETHA_MODE_DEFAULT, &channel)) != SUBETHA_OK)
        return fail("open", rc);
    /* A buffer of SUBETHA_CHANNEL_ITEM_MAX bytes holds any item, and its
       length comes back beside it. SUBETHA_E_RING_EMPTY means every item
       has been taken. */
    uint8_t item[SUBETHA_CHANNEL_ITEM_MAX];
    size_t len;
    while ((rc = subetha_channel_try_recv(channel, item, sizeof item, &len)) == SUBETHA_OK)
        printf("received %.*s\n", (int)len, (const char *)item);
    if (rc != SUBETHA_E_RING_EMPTY) return fail("receive", rc);
    if ((rc = subetha_handle_destroy(channel)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
received first
received second
received third
```

## Wait for an item

`recv` answers at once, with an item or with nothing. A reader with
nothing else to do waits instead: the waiting form sleeps until an item
lands or the time it was given runs out. These two programs use the
channel the sender made. The waiter starts first and waits for three
items, and the teller sends them a fifth of a second apart, so each one
lands while the waiter sleeps.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/waiter.rs" alongside="true"}
use std::time::Duration;

use subetha::cxc::{Channel, SlotValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Channel::<SlotValue>::open("items.chan", 16)?;
    for _ in 0..3 {
        // Sleep until an item lands, for ten seconds at most.
        let item = channel.recv_blocking(Some(Duration::from_secs(10)))?;
        println!("waiter: {}", String::from_utf8_lossy(item.as_bytes()));
    }
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="waiter.py" alongside="true"}
import subetha

channel = subetha.Channel.open("items.chan", capacity=16)
for _ in range(3):
    # Sleep until an item lands, for ten seconds at most; None means the
    # time ran out. Other Python threads run while it sleeps.
    item = channel.recv_for(timeout=10)
    if item is None:
        raise RuntimeError("no item came")
    print("waiter:", item.decode())
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="waiter.ps1" alongside="true"}
Import-Module SubEtha

$channel = Open-SubEthaChannel -Path items.chan -Capacity 16
foreach ($i in 1..3) {
    # Sleep until an item lands, for ten seconds at most; $null means the
    # time ran out.
    $item = $channel.RecvFor(10)
    if ($null -eq $item) { throw 'no item came' }
    "waiter: $([Text.Encoding]::UTF8.GetString($item))"
}
$channel.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="waiter.c" alongside="true"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle channel;
    if ((rc = subetha_channel_open("items.chan", 16, SUBETHA_MODE_DEFAULT, &channel)) != SUBETHA_OK)
        return fail("open", rc);
    uint8_t item[SUBETHA_CHANNEL_ITEM_MAX];
    size_t len;
    for (int i = 0; i < 3; i++) {
        /* Sleep until an item lands, for ten seconds at most;
           SUBETHA_E_TIMEOUT means the time ran out. */
        if ((rc = subetha_channel_recv_wait(channel, item, sizeof item, &len, 10000)) != SUBETHA_OK)
            return fail("wait", rc);
        printf("waiter: %.*s\n", (int)len, (const char *)item);
    }
    if ((rc = subetha_handle_destroy(channel)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
waiter: one
waiter: two
waiter: three
```

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/teller.rs"}
use std::time::Duration;

use subetha::cxc::{Channel, SlotValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Channel::<SlotValue>::open("items.chan", 16)?;
    for word in ["one", "two", "three"] {
        std::thread::sleep(Duration::from_millis(200));
        channel.send(&SlotValue::from_bytes(word.as_bytes())?)?;
    }
    println!("teller: sent 3 items");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="teller.py"}
import time

import subetha

channel = subetha.Channel.open("items.chan", capacity=16)
for word in [b"one", b"two", b"three"]:
    time.sleep(0.2)
    if not channel.send(word):
        raise RuntimeError("the channel is full")
print("teller: sent 3 items")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="teller.ps1"}
Import-Module SubEtha

$channel = Open-SubEthaChannel -Path items.chan -Capacity 16
foreach ($word in 'one', 'two', 'three') {
    Start-Sleep -Milliseconds 200
    if (-not $channel.Send($word)) { throw 'the channel is full' }
}
$channel.Dispose()
'teller: sent 3 items'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="teller.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#endif

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

/* Wait a fifth of a second. */
static void pause_a_fifth(void) {
#if defined(_WIN32)
    Sleep(200);
#else
    struct timespec ts = {0, 200000000};
    nanosleep(&ts, NULL);
#endif
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle channel;
    if ((rc = subetha_channel_open("items.chan", 16, SUBETHA_MODE_DEFAULT, &channel)) != SUBETHA_OK)
        return fail("open", rc);
    const char *words[] = {"one", "two", "three"};
    for (int i = 0; i < 3; i++) {
        pause_a_fifth();
        rc = subetha_channel_try_send(channel, (const uint8_t *)words[i], strlen(words[i]));
        if (rc != SUBETHA_OK) return fail("send", rc);
    }
    if ((rc = subetha_handle_destroy(channel)) != SUBETHA_OK) return fail("destroy", rc);
    printf("teller: sent 3 items\n");
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
teller: sent 3 items
```

The waiting forms are `recv_blocking` in Rust, `recv_for` in Python,
`RecvFor` in PowerShell and `subetha_channel_recv_wait` in C, and a send
waits for room the same way when the channel is full: `send_blocking`,
`send_for`, `SendFor` and `subetha_channel_send_wait`. A wait costs
nothing while it sleeps. In C, destroying the handle from another thread
ends a wait with `SUBETHA_E_DESTROYED`.

## A ring with a producer and a consumer

A ring is the structure under the queues the C ABI started with. A
sender registers as a producer and a reader as a consumer, each gets an
id, and every send and receive names it. Making a ring attaches to it
when another process has already made it, so both programs below make it
and either can start first. The worker waits for three jobs, and the
boss sends them.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/worker.rs" alongside="true"}
use std::sync::Arc;
use std::time::Duration;

use subetha::cxc::{AdaptiveRing, BlockingAdaptiveRing, ADAPTIVE_SPSC_PAYLOAD_BYTES, MAX_WAITERS_DEFAULT};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 16 slots for one producer and one consumer, made when they are not
    // there and attached to when they are.
    let ring = Arc::new(AdaptiveRing::create("jobs", 1, 1, 16)?);
    // The wakers beside the ring are what a waiting receive sleeps on.
    let jobs = BlockingAdaptiveRing::beside_files(Arc::clone(&ring), "jobs", MAX_WAITERS_DEFAULT)?;
    let consumer = ring.register_consumer()?;
    let mut slot = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
    for _ in 0..3 {
        jobs.recv_blocking(consumer, &mut slot, Some(Duration::from_secs(10)))?;
        // A slot comes back whole: the job is the bytes before the zeros.
        let end = slot.iter().position(|&b| b == 0).unwrap_or(slot.len());
        println!("worker: {}", String::from_utf8_lossy(&slot[..end]));
    }
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="worker.py" alongside="true"}
import subetha

# 16 slots for one producer and one consumer, made when they are not there
# and attached to when they are.
ring = subetha.Ring("jobs", 16)
consumer = ring.register_consumer()
for _ in range(3):
    job = ring.recv_for(consumer, timeout=10)
    if job is None:
        raise RuntimeError("no job came")
    # A slot comes back whole: the job is the bytes before the zeros.
    print("worker:", job.rstrip(b"\0").decode())
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="worker.ps1" alongside="true"}
Import-Module SubEtha

# 16 slots for one producer and one consumer, made when they are not there
# and attached to when they are.
$ring = New-SubEthaRing -Path jobs -Capacity 16
$consumer = $ring.RegisterConsumer()
foreach ($i in 1..3) {
    $job = $ring.RecvFor($consumer, 10)
    if ($null -eq $job) { throw 'no job came' }
    # A slot comes back whole: the job is the text before the zeros.
    "worker: $([Text.Encoding]::UTF8.GetString($job).TrimEnd([char]0))"
}
$ring.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="worker.c" alongside="true"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

static const subetha_ring_options options = {.mode = SUBETHA_MODE_STRICT, .stamps = SUBETHA_STAMPS_NONE};

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* 16 slots for one producer and one consumer, made when they are not
       there and attached to when they are. */
    subetha_handle ring;
    if ((rc = subetha_ring_create("jobs", 1, 1, 16, &options, &ring)) != SUBETHA_OK) return fail("ring", rc);
    uint32_t consumer;
    if ((rc = subetha_ring_register_consumer(ring, &consumer)) != SUBETHA_OK) return fail("consumer", rc);
    uint8_t slot[SUBETHA_RING_SLOT_BYTES];
    size_t len;
    for (int i = 0; i < 3; i++) {
        if ((rc = subetha_ring_pop_wait(ring, consumer, slot, sizeof slot, &len, 10000)) != SUBETHA_OK)
            return fail("wait", rc);
        /* A slot comes back whole: the job is the bytes before the zeros. */
        const uint8_t *end = memchr(slot, 0, len);
        printf("worker: %.*s\n", end ? (int)(end - slot) : (int)len, (const char *)slot);
    }
    if ((rc = subetha_handle_destroy(ring)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
worker: job 1
worker: job 2
worker: job 3
```

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/boss.rs"}
use std::sync::Arc;
use std::time::Duration;

use subetha::cxc::{AdaptiveRing, BlockingAdaptiveRing, MAX_WAITERS_DEFAULT};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ring = Arc::new(AdaptiveRing::create("jobs", 1, 1, 16)?);
    let jobs = BlockingAdaptiveRing::beside_files(Arc::clone(&ring), "jobs", MAX_WAITERS_DEFAULT)?;
    let producer = ring.register_producer()?;
    for n in 1..=3 {
        // A send through the wakers wakes a receive waiting in any process.
        jobs.send_blocking(producer, format!("job {n}").as_bytes(), Some(Duration::from_secs(10)))?;
    }
    println!("boss: sent 3 jobs");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="boss.py"}
import subetha

ring = subetha.Ring("jobs", 16)
producer = ring.register_producer()
for n in range(1, 4):
    # send_for waits for room while the ring is full; False means the
    # time ran out.
    if not ring.send_for(producer, f"job {n}".encode(), timeout=10):
        raise RuntimeError("the ring stayed full")
print("boss: sent 3 jobs")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="boss.ps1"}
Import-Module SubEtha

$ring = New-SubEthaRing -Path jobs -Capacity 16
$producer = $ring.RegisterProducer()
foreach ($n in 1..3) {
    # SendFor waits for room while the ring is full; False means the time
    # ran out.
    if (-not $ring.SendFor($producer, "job $n", 10)) { throw 'the ring stayed full' }
}
$ring.Dispose()
'boss: sent 3 jobs'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="boss.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

static const subetha_ring_options options = {.mode = SUBETHA_MODE_STRICT, .stamps = SUBETHA_STAMPS_NONE};

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle ring;
    if ((rc = subetha_ring_create("jobs", 1, 1, 16, &options, &ring)) != SUBETHA_OK) return fail("ring", rc);
    uint32_t producer;
    if ((rc = subetha_ring_register_producer(ring, &producer)) != SUBETHA_OK) return fail("producer", rc);
    char job[16];
    for (int n = 1; n <= 3; n++) {
        int len = snprintf(job, sizeof job, "job %d", n);
        /* Waits for room while the ring is full, for ten seconds at most. */
        rc = subetha_ring_push_wait(ring, producer, (const uint8_t *)job, (size_t)len, 10000);
        if (rc != SUBETHA_OK) return fail("send", rc);
    }
    if ((rc = subetha_handle_destroy(ring)) != SUBETHA_OK) return fail("destroy", rc);
    printf("boss: sent 3 jobs\n");
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
boss: sent 3 jobs
```

A ring hands back a whole slot, 64 bytes on this one, so a reader that
wants an item's own length carries it inside the item. A channel keeps
the length for it, which is why the channel's readers above print each
item as it was sent.

## Run them

Run each surface's programs from the folder that holds them, starting
each waiting program and, while it runs, the program it waits for. The
`sh` lines are for a POSIX shell and the `PS>` lines for PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin send
cargo run --bin receive
cargo run --bin waiter & cargo run --bin teller; wait
cargo run --bin worker & cargo run --bin boss; wait
```
```powershell
PS> cargo run --bin send
PS> cargo run --bin receive
PS> $waiter = Start-Process cargo -ArgumentList 'run', '--bin', 'waiter' -NoNewWindow -PassThru
PS> cargo run --bin teller
PS> $waiter.WaitForExit()
PS> $worker = Start-Process cargo -ArgumentList 'run', '--bin', 'worker' -NoNewWindow -PassThru
PS> cargo run --bin boss
PS> $worker.WaitForExit()
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python send.py
python receive.py
python waiter.py & python teller.py; wait
python worker.py & python boss.py; wait
```
```powershell
PS> python send.py
PS> python receive.py
PS> $waiter = Start-Process python -ArgumentList 'waiter.py' -NoNewWindow -PassThru
PS> python teller.py
PS> $waiter.WaitForExit()
PS> $worker = Start-Process python -ArgumentList 'worker.py' -NoNewWindow -PassThru
PS> python boss.py
PS> $worker.WaitForExit()
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File send.ps1
PS> pwsh -File receive.ps1
PS> $waiter = Start-Process pwsh -ArgumentList '-File', 'waiter.ps1' -NoNewWindow -PassThru
PS> pwsh -File teller.ps1
PS> $waiter.WaitForExit()
PS> $worker = Start-Process pwsh -ArgumentList '-File', 'worker.ps1' -NoNewWindow -PassThru
PS> pwsh -File boss.ps1
PS> $worker.WaitForExit()
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
for p in send receive waiter teller worker boss; do
  cc $p.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o $p
done
./send
./receive
./waiter & ./teller; wait
./worker & ./boss; wait
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs, which also gives the CMake build and the Windows libraries.

## What happened

The channel lives in `items.chan`, with the wakers its waits sleep on
beside it in `items.chan.cw` and `items.chan.pw`, and the files stay
after every program ends. Every surface writes an item the same way, its
length and then its bytes, so a channel one surface made is the same
channel to the others: a Python sender and a C reader can share one.

Making a channel and making a ring differ in one way that decides the
order programs start in. Making a channel starts its file afresh, items
and all, so one process makes it and the rest join it with `open`.
Making a ring attaches to the one already there, as making an atomic
does in [Share a counter](../share-a-counter/), so every process can
make it and the first to arrive builds it.

## What to do next

- [Share a counter](../share-a-counter/) adds to one number from two
  processes with no lock.
- [Move many items per call](../move-many-items/) sends and receives a
  whole run of items in one call.
- The [reference](../../reference/subetha-cxc/) lists every queue and
  ring with its layout and operations.
