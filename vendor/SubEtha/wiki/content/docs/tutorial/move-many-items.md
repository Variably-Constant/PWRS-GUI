---
title: "Move many items per call"
weight: 35
---

A call from Python, PowerShell or C into SubEtha crosses from one
language into another, and the crossing costs time of its own however
little the call does. On a Ryzen 9 7900X a call costs about 30
nanoseconds from Python, about 7 from C, and from 650 to 1,900 from
PowerShell depending on the host. So each of those surfaces has calls
that take a whole run of items at once, and a program that moves many
items moves them through those. Rust calls the structures directly with
no crossing, so a loop is already the cheapest shape there. Save each
surface's programs in one folder and run them from it in order.

## A run of items in one call

The program makes a ring of 1024 slots for one producer and one
consumer, sends the numbers 0 to 999 as 8-byte items, takes them back,
and adds them up.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/batch.rs"}
use subetha::cxc::{AdaptiveRing, RingError, ADAPTIVE_SPSC_PAYLOAD_BYTES};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A ring of 1024 slots for one producer and one consumer.
    let ring = AdaptiveRing::create("batch", 1, 1, 1024)?;
    let producer = ring.register_producer()?;
    let consumer = ring.register_consumer()?;

    // Rust calls the ring itself, so a loop crosses no boundary.
    let mut sent = 0;
    for i in 0..1000u64 {
        ring.try_send(producer, &i.to_le_bytes())?;
        sent += 1;
    }
    // A slot comes back whole; the item is its first 8 bytes.
    let (mut received, mut sum) = (0, 0u64);
    let mut slot = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
    loop {
        match ring.try_recv(consumer, &mut slot) {
            Ok(_) => {
                received += 1;
                sum += u64::from_le_bytes(slot[..8].try_into()?);
            }
            Err(RingError::Empty) => break,
            Err(e) => return Err(e.into()),
        }
    }
    println!("sent {sent}");
    println!("received {received}, sum {sum}");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="batch.py"}
import subetha

# A ring of 1024 slots for one producer and one consumer.
ring = subetha.Ring("batch", 1024)
producer = ring.register_producer()
consumer = ring.register_consumer()

# One call carries the whole run each way. send_many stops at the first
# item the ring refuses and answers how many went in.
sent = ring.send_many(producer, [i.to_bytes(8, "little") for i in range(1000)])
items = ring.recv_many(consumer, 1000)
# A slot comes back whole; the item is its first 8 bytes.
total = sum(int.from_bytes(item[:8], "little") for item in items)
print("sent", sent)
print(f"received {len(items)}, sum {total}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="batch.ps1"}
Import-Module SubEtha

# A ring of 1024 slots for one producer and one consumer.
$ring = New-SubEthaRing -Path batch -Capacity 1024
$producer = $ring.RegisterProducer()
$consumer = $ring.RegisterConsumer()

# One call carries the whole run each way. The comma keeps each byte[] one
# item rather than letting PowerShell spread it into its bytes.
$items = @(foreach ($i in 0..999) { , [BitConverter]::GetBytes([uint64]$i) })
$sent = $ring.SendMany($producer, $items)
$received = $ring.RecvMany($consumer, 1000)
# A slot comes back whole; the item is its first 8 bytes.
$sum = [uint64]0
foreach ($item in $received) { $sum += [BitConverter]::ToUInt64($item, 0) }
"sent $sent"
"received $($received.Count), sum $sum"
$ring.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="batch.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
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

    /* A ring of 1024 slots for one producer and one consumer. */
    subetha_handle ring;
    if ((rc = subetha_ring_create("batch", 1, 1, 1024, &options, &ring)) != SUBETHA_OK)
        return fail("create", rc);
    uint32_t producer, consumer;
    if ((rc = subetha_ring_register_producer(ring, &producer)) != SUBETHA_OK) return fail("producer", rc);
    if ((rc = subetha_ring_register_consumer(ring, &consumer)) != SUBETHA_OK) return fail("consumer", rc);

    /* One call carries the whole run each way: the items lie one after
       another, stride bytes apart. */
    uint64_t items[1000];
    for (uint64_t i = 0; i < 1000; i++) items[i] = i;
    size_t sent;
    rc = subetha_ring_try_push_many(ring, producer, (const uint8_t *)items, sizeof items[0],
                                    sizeof items[0], 1000, &sent);
    if (rc != SUBETHA_OK) return fail("push", rc);

    /* A slot comes back whole, SUBETHA_RING_SLOT_BYTES apart; the item is
       its first 8 bytes. */
    uint8_t *slots = malloc((size_t)1000 * SUBETHA_RING_SLOT_BYTES);
    if (slots == NULL) return 1;
    size_t received;
    rc = subetha_ring_try_pop_many(ring, consumer, slots, SUBETHA_RING_SLOT_BYTES, 1000, &received);
    if (rc != SUBETHA_OK) return fail("pop", rc);
    uint64_t sum = 0;
    for (size_t i = 0; i < received; i++) {
        uint64_t value;
        memcpy(&value, slots + i * SUBETHA_RING_SLOT_BYTES, sizeof value);
        sum += value;
    }
    free(slots);
    printf("sent %zu\n", sent);
    printf("received %zu, sum %llu\n", received, (unsigned long long)sum);

    if ((rc = subetha_handle_destroy(ring)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
sent 1000
received 1000, sum 499500
```

A run's call stops at the first item it cannot place and answers how
many it moved: a full ring ends a send early, an empty one a receive, and
the program comes back with the rest. The sending call is `send_many` in
Python, `SendMany` in PowerShell and `subetha_ring_try_push_many` in C,
and the receiving one `recv_many`, `RecvMany` and
`subetha_ring_try_pop_many`. The other families have run forms beside
their single-item calls in the same way, such as `insert_many` and
`get_many` on a map.

On the same machine, PowerShell 7 moving items through a ring with one
`Push` and one `Pop` each costs about 4,800 nanoseconds an item, and
with 256 items per `PushMany` and `PopMany` about 400. Python batched a
thousand at a time costs about 1.3 nanoseconds an item.
`bench/call_shapes.py` in the Python package and `bench/CallShapes.ps1`
in the PowerShell module measure the same shapes on your own machine.

## Items packed end to end

A run still makes one object per item in Python and PowerShell, and
building those objects costs more than the crossing once the crossing is
shared. Packed into one buffer, a run makes none: the sending call cuts
the buffer into items itself, and the receiving call hands them back in
one buffer, each item in a whole slot. In C the items are always packed,
and `stride` lets the call take a field out of each record of an array;
in Rust the items are slices of one buffer.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/packed.rs"}
use subetha::cxc::{AdaptiveRing, RingError, ADAPTIVE_SPSC_PAYLOAD_BYTES};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ring = AdaptiveRing::create("packed", 1, 1, 1024)?;
    let producer = ring.register_producer()?;
    let consumer = ring.register_consumer()?;

    // 1000 items of 8 bytes, end to end in one buffer.
    let data: Vec<u8> = (0..1000u64).flat_map(u64::to_le_bytes).collect();
    let mut sent = 0;
    for item in data.chunks(8) {
        ring.try_send(producer, item)?;
        sent += 1;
    }
    // Back into one buffer, each item in a whole slot of it.
    let mut packed = vec![0u8; 1000 * ADAPTIVE_SPSC_PAYLOAD_BYTES];
    let mut received = 0;
    for slot in packed.chunks_mut(ADAPTIVE_SPSC_PAYLOAD_BYTES) {
        match ring.try_recv(consumer, slot) {
            Ok(_) => received += 1,
            Err(RingError::Empty) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let mut sum = 0u64;
    for slot in packed.chunks(ADAPTIVE_SPSC_PAYLOAD_BYTES).take(received) {
        sum += u64::from_le_bytes(slot[..8].try_into()?);
    }
    println!("sent {sent}");
    println!("received {received}, sum {sum}");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="packed.py"}
from array import array

import subetha

ring = subetha.Ring("packed", 1024)
producer = ring.register_producer()
consumer = ring.register_consumer()

# 1000 unsigned 8-byte items, end to end in one buffer. send_buffer cuts
# it into items of the length given, with no Python object per item.
data = array("Q", range(1000)).tobytes()
sent = ring.send_buffer(producer, data, 8)
# recv_buffer hands them back the same way, each in a whole slot; every
# slot's first 8 bytes are read through one view of the buffer.
packed, received = ring.recv_buffer(consumer, 1000)
slot = len(packed) // received
total = sum(memoryview(packed).cast("Q")[:: slot // 8])
print("sent", sent)
print(f"received {received}, sum {total}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="packed.ps1"}
Import-Module SubEtha

$ring = New-SubEthaRing -Path packed -Capacity 1024
$producer = $ring.RegisterProducer()
$consumer = $ring.RegisterConsumer()

# 1000 items of 8 bytes, end to end in one byte[]. SendPacked cuts it into
# items of the length given, with no object per item.
$data = [byte[]]::new(8000)
foreach ($i in 0..999) { [BitConverter]::GetBytes([uint64]$i).CopyTo($data, $i * 8) }
$sent = $ring.SendPacked($producer, $data, 8)
# RecvPacked hands them back the same way, each in a whole slot of one
# byte[]; an item is its slot's first 8 bytes.
$packed = $ring.RecvPacked($consumer, 1000)
$slot = $packed.Bytes.Length / $packed.Count
$sum = [uint64]0
foreach ($i in 0..($packed.Count - 1)) { $sum += [BitConverter]::ToUInt64($packed.Bytes, $i * $slot) }
"sent $sent"
"received $($packed.Count), sum $sum"
$ring.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="packed.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

static const subetha_ring_options options = {.mode = SUBETHA_MODE_STRICT, .stamps = SUBETHA_STAMPS_NONE};

/* A record of which only the reading travels. */
struct reading {
    uint64_t value;
    uint64_t taken_at;
};

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle ring;
    if ((rc = subetha_ring_create("packed", 1, 1, 1024, &options, &ring)) != SUBETHA_OK)
        return fail("create", rc);
    uint32_t producer, consumer;
    if ((rc = subetha_ring_register_producer(ring, &producer)) != SUBETHA_OK) return fail("producer", rc);
    if ((rc = subetha_ring_register_consumer(ring, &consumer)) != SUBETHA_OK) return fail("consumer", rc);

    /* Each item is the value field, 8 bytes, and the next one starts a
       whole record on. */
    static struct reading readings[1000];
    for (uint64_t i = 0; i < 1000; i++) readings[i] = (struct reading){.value = i, .taken_at = 0};
    size_t sent;
    rc = subetha_ring_try_push_many(ring, producer, (const uint8_t *)&readings[0].value,
                                    sizeof readings[0], sizeof readings[0].value, 1000, &sent);
    if (rc != SUBETHA_OK) return fail("push", rc);

    uint8_t *slots = malloc((size_t)1000 * SUBETHA_RING_SLOT_BYTES);
    if (slots == NULL) return 1;
    size_t received;
    rc = subetha_ring_try_pop_many(ring, consumer, slots, SUBETHA_RING_SLOT_BYTES, 1000, &received);
    if (rc != SUBETHA_OK) return fail("pop", rc);
    uint64_t sum = 0;
    for (size_t i = 0; i < received; i++) {
        uint64_t value;
        memcpy(&value, slots + i * SUBETHA_RING_SLOT_BYTES, sizeof value);
        sum += value;
    }
    free(slots);
    printf("sent %zu\n", sent);
    printf("received %zu, sum %llu\n", received, (unsigned long long)sum);

    if ((rc = subetha_handle_destroy(ring)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
sent 1000
received 1000, sum 499500
```

Packed, 256 items each way cost PowerShell 7 about 39 nanoseconds an
item on the Ryzen 9 7900X, against 400 for the same run as objects.
Python's `send_buffer` and PowerShell's `SendPacked` take the buffer and
the item length; a buffer whose length is not a whole number of items
sends the last item short. `recv_buffer` answers the buffer and how many
items it holds, and `RecvPacked` an object with `Bytes` and `Count`; a
slot on this ring is 64 bytes.

## Run them

Run each surface's programs from the folder that holds them. The `sh`
lines are for a POSIX shell and the `PS>` lines for PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin batch
cargo run --bin packed
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python batch.py
python packed.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File batch.ps1
PS> pwsh -File packed.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc batch.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o batch
cc packed.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o packed
./batch
./packed
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs. Each ring lives in files named after it, `batch.spsc.bin` and
the rest beside it, which stay after the program ends.

## What to do next

- [Read a failure](../read-a-failure/) reads what each surface reports
  when a call is refused.
- The [reference](../../reference/subetha-cxc/) lists the run form of
  every structure.
