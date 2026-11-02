---
title: "Blocking Adaptive Ring"
weight: 22
---

An [adaptive ring](../shared-ring-adaptive/) every process can wait on.
`BlockingAdaptiveRing` keeps the ring together with the two wakers its
waiters park on and the notify record its pollers attach to: a push
through it wakes one waiting consumer and signals every attached
notifier, and a pop through it wakes one waiting producer. The C ABI's
ring, Python's `Ring` and PowerShell's `SubEtha.Ring` all push and wait
through it, so a wait on any surface wakes on a push from any surface.

## Waiting for an item

Each program waits on an empty ring until its timeout, then sends an
item and waits again, which returns at once.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/main.rs"}
use std::sync::Arc;
use std::time::Duration;

use subetha::cxc::blocking_adaptive_ring::MAX_WAITERS_DEFAULT;
use subetha::cxc::blocking_spsc_ring::BlockingError;
use subetha::cxc::{AdaptiveRing, BlockingAdaptiveRing};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The ring, with its wakers and notify record beside it under the
    // names every surface finds.
    let ring = Arc::new(AdaptiveRing::create("ring", 1, 1, 64)?);
    let ring = BlockingAdaptiveRing::beside_files(ring, "ring", MAX_WAITERS_DEFAULT)?;
    let producer = ring.ring().register_producer()?;
    let consumer = ring.ring().register_consumer()?;
    let mut out = [0u8; 64];
    match ring.recv_blocking(consumer, &mut out, Some(Duration::from_millis(50))) {
        Err(BlockingError::Timeout) => println!("nothing within 50 ms"),
        other => return Err(format!("an empty wait answered {other:?}").into()),
    }
    ring.try_send(producer, b"hello")?;
    ring.recv_blocking(consumer, &mut out, Some(Duration::from_secs(5)))?;
    println!("received: {}", String::from_utf8_lossy(&out[..5]));
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="wait.py"}
import subetha

ring = subetha.Ring("ring", 64)
producer = ring.register_producer()
consumer = ring.register_consumer()
if ring.recv_for(consumer, timeout=0.05) is None:
    print("nothing within 50 ms")
ring.send(producer, b"hello")
item = ring.recv_for(consumer, timeout=5)
print("received:", item[:5].decode())
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="wait.ps1"}
Import-Module SubEtha

$ring = New-SubEthaRing -Path ring -Capacity 64
$producer = $ring.RegisterProducer()
$consumer = $ring.RegisterConsumer()
if ($null -eq $ring.RecvFor($consumer, 0.05)) { 'nothing within 50 ms' }
$null = $ring.Send($producer, 'hello')
$item = $ring.RecvFor($consumer, 5)
"received: $([System.Text.Encoding]::UTF8.GetString($item, 0, 5))"
$ring.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="wait.c"}
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
    if ((rc = subetha_ring_create("ring", 1, 1, 64, &options, &ring)) != SUBETHA_OK)
        return fail("create", rc);
    uint32_t producer, consumer;
    if ((rc = subetha_ring_register_producer(ring, &producer)) != SUBETHA_OK) return fail("producer", rc);
    if ((rc = subetha_ring_register_consumer(ring, &consumer)) != SUBETHA_OK) return fail("consumer", rc);

    uint8_t out[SUBETHA_RING_SLOT_BYTES];
    size_t len;
    rc = subetha_ring_pop_wait(ring, consumer, out, sizeof out, &len, 50);
    if (rc != SUBETHA_E_TIMEOUT) return fail("an empty wait", rc);
    printf("nothing within 50 ms\n");
    if ((rc = subetha_ring_try_push(ring, producer, (const uint8_t *)"hello", 5)) != SUBETHA_OK)
        return fail("push", rc);
    if ((rc = subetha_ring_pop_wait(ring, consumer, out, sizeof out, &len, 5000)) != SUBETHA_OK)
        return fail("pop", rc);
    printf("received: %.5s\n", (const char *)out);

    if ((rc = subetha_handle_destroy(ring)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
nothing within 50 ms
received: hello
```

## Where the wakers live

The wakers and the notify record sit beside the ring under fixed names,
which is how every process and every surface finds the same ones:

| The ring | Its wakers and notify record |
|---|---|
| file-backed, under `<prefix>` | `<prefix>.cwaker.bin`, `<prefix>.pwaker.bin`, `<prefix>.notify.bin` |
| in named shared memory `{name}` | `{name}_cwaker`, `{name}_pwaker`, `{name}_notify` |
| anonymous | in the process that made it, and nowhere else |

Every process names the same number of waiter slots: Rust's
`MAX_WAITERS_DEFAULT`, which is what Python and PowerShell use and what
C uses when `subetha_ring_options` leaves `max_waiters` at zero, unless
every process agrees on another. A waker made with another number is
refused as a layout mismatch.

## The operations on each surface

| Rust | Python | PowerShell | C | What it does |
|---|---|---|---|---|
| `try_send` | `send` | `Send` | `subetha_ring_try_push` | sends without waiting, then wakes one waiting consumer and signals every notifier |
| `send_blocking` | `send_for` | `SendFor` | `subetha_ring_push_wait` | sends, waiting for room |
| `try_recv` | `recv` | `Recv` | `subetha_ring_try_pop` | receives without waiting, then wakes one waiting producer |
| `recv_blocking` | `recv_for` | `RecvFor` | `subetha_ring_pop_wait` | receives, waiting for an item |
| `close` | | | `subetha_handle_destroy` | ends every wait through that handle |

On a timeout Rust returns `BlockingError::Timeout`, Python answers
`False` or `None`, PowerShell `$false` or `$null`, and C returns
`SUBETHA_E_TIMEOUT`. Rust takes the timeout as a `Duration`, C in
milliseconds, and Python and PowerShell in seconds. A push or pop made
through the
`AdaptiveRing` itself, rather than through `BlockingAdaptiveRing`,
wakes no one.
