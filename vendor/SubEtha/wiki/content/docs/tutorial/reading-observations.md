---
title: "Reading observations"
weight: 40
---

The sidecar is a thread in your process that watches the structures you
register with it. Each operation on a registered structure leaves a
small record, and the sidecar folds the records into counts you can read
while the program runs. This page registers a hash map, works it, and
reads what the sidecar counted. Save the program for your surface in a
folder and run it from there.

## Observe a map

The program makes a map with room for 2048 entries, twice the 1000 it
will hold, and registers it with the sidecar. It inserts 1000 keys,
reads each one back, has the sidecar drain what was recorded, and prints
what the sidecar counted.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/observe.rs"}
use subetha::cxc::SharedHashMap;
use subetha::sidecar::{global, SidecarBox};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Room for 2048 entries, twice what it will hold. Wrapping the map in
    // a SidecarBox registers it with the process's sidecar until the box
    // drops.
    let map = SidecarBox::new(SharedHashMap::<u32, u64>::create("observed.map", 2048)?);
    for key in 0..1000u32 {
        map.insert(key, u64::from(key))?;
        map.get(&key).ok_or("a key went missing")?;
    }
    // Drain what was recorded now, rather than at the sidecar's next scan.
    global().scan_now();
    let stats = map.stats().ok_or("the map is not registered")?;
    println!("observed {} operations", stats.ops_observed);
    println!("inserts: {}", stats.op_kind_counts[1]);
    println!("gets: {}", stats.op_kind_counts[2]);
    println!("contended: {}", stats.contention_ops);
    println!("latency ticks: {}", stats.total_latency_ticks);
    println!("migrations: {}", stats.migrations_triggered);
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="observe.py"}
import subetha
from subetha import sidecar

# Room for 2048 entries of 4-byte keys and 8-byte values, twice what it
# will hold.
table = subetha.HashMap("observed.map", 2048, 4, 8)
# observe registers the map with the process's sidecar until the with
# block ends.
with table.observe() as registration:
    for key in range(1000):
        k = key.to_bytes(4, "little")
        # insert answers None when the map is full.
        if table.insert(k, key.to_bytes(8, "little")) is None:
            raise RuntimeError("the map is full")
        if table.get(k) is None:
            raise RuntimeError("a key went missing")
    # Drain what was recorded now, rather than at the sidecar's next scan.
    sidecar.scan_now()
    stats = registration.stats()
print("observed", stats.ops_observed, "operations")
print("inserts:", stats.op_kind_counts[1])
print("gets:", stats.op_kind_counts[2])
print("contended:", stats.contention_ops)
print("latency ticks:", stats.total_latency_ticks)
print("migrations:", stats.migrations_triggered)
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="observe.ps1"}
Import-Module SubEtha

# Room for 2048 entries of 4-byte keys and 8-byte values, twice what it
# will hold.
$map = New-SubEthaHashMap -Path observed.map -Capacity 2048 -KeySize 4 -ValueSize 8
# Observe registers the map with the process's sidecar until Close.
$registration = $map.Observe()
foreach ($key in 0..999) {
    $k = [BitConverter]::GetBytes([uint32]$key)
    $answer = $map.Insert($k, [BitConverter]::GetBytes([uint64]$key))
    if ($answer -eq [SubEtha.InsertOutcome]::Full) { throw 'the map is full' }
    if ($null -eq $map.Get($k)) { throw 'a key went missing' }
}
# Drain what was recorded now, rather than at the sidecar's next scan.
Invoke-SubEthaSidecarScan
$stats = $registration.Stats()
$registration.Close()
$map.Dispose()
"observed $($stats.OpsObserved) operations"
"inserts: $($stats.OpKindCounts()[1])"
"gets: $($stats.OpKindCounts()[2])"
"contended: $($stats.ContentionOps)"
"latency ticks: $($stats.TotalLatencyTicks)"
"migrations: $($stats.MigrationsTriggered)"
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="observe.c"}
#include "subetha.h"
#include <inttypes.h>
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

    /* Room for 2048 entries of 4-byte keys and 8-byte values, twice what
       it will hold. */
    subetha_handle map;
    if ((rc = subetha_hashmap_create("observed.map", 2048, 4, 8, SUBETHA_MODE_DEFAULT, &map)) != SUBETHA_OK)
        return fail("create", rc);
    /* Register the map with the process's sidecar until its handle is
       destroyed. */
    if ((rc = subetha_observe(map)) != SUBETHA_OK) return fail("observe", rc);
    for (uint32_t key = 0; key < 1000; key++) {
        uint64_t value = key;
        uint8_t out[8];
        size_t len;
        rc = subetha_hashmap_insert(map, (const uint8_t *)&key, sizeof key, (const uint8_t *)&value, sizeof value, NULL);
        if (rc != SUBETHA_OK) return fail("insert", rc);
        rc = subetha_hashmap_get(map, (const uint8_t *)&key, sizeof key, out, sizeof out, &len);
        if (rc != SUBETHA_OK) return fail("get", rc);
    }
    /* Drain what was recorded now, rather than at the sidecar's next
       scan. */
    if ((rc = subetha_sidecar_scan_now()) != SUBETHA_OK) return fail("scan", rc);
    subetha_instance_stats stats;
    if ((rc = subetha_observation_stats(map, &stats)) != SUBETHA_OK) return fail("stats", rc);
    if ((rc = subetha_handle_destroy(map)) != SUBETHA_OK) return fail("destroy", rc);
    printf("observed %" PRIu64 " operations\n", stats.ops_observed);
    printf("inserts: %" PRIu64 "\n", stats.op_kind_counts[1]);
    printf("gets: %" PRIu64 "\n", stats.op_kind_counts[2]);
    printf("contended: %" PRIu64 "\n", stats.contention_ops);
    printf("latency ticks: %" PRIu64 "\n", stats.total_latency_ticks);
    printf("migrations: %" PRIu64 "\n", stats.migrations_triggered);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
observed 2000 operations
inserts: 1000
gets: 1000
contended: 0
latency ticks: 0
migrations: 0
```

Registering is one call on the structure: Rust wraps it in a
`SidecarBox`, Python and PowerShell call its `observe` method, and C
calls `subetha_observe` on its handle. The registration ends when the
`SidecarBox` drops, when the Python `with` block ends, when PowerShell's
`Close` runs, or when the C handle is destroyed.

## What the counts say

Every operation on a registered structure records one observation, 24
bytes naming the kind of operation, two flags, a latency and the thread
that made it. Each structure numbers its own kinds of operation. A hash
map counts an insert as kind 1, a get as 2, a remove as 3, a contains
as 4, a clear as 5 and a compact as 6, which is why the program reads
the inserts at index 1 and the gets at index 2. Kind 0 means
unspecified, and kinds 7 and above share the last count.

The first flag marks contention. A hash map sets it on an insert that
finds the table full, and `contended` counts the operations that carried
it. The latency is the time an operation took, in timestamp-counter
ticks; the structures SubEtha ships record none, so it stays zero.
`migrations` counts the scans whose policy chose another way for the
structure to run, and a hash map's policy never does.

## Run it

Run the program for your surface from the folder that holds it. The `sh`
lines are for a POSIX shell and the `PS>` lines for PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin observe
```
```powershell
PS> cargo run --bin observe
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python observe.py
```
```powershell
PS> python observe.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File observe.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc observe.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o observe
./observe
```
{{< /tab >}}

{{< /tabs >}}

The Rust program is a binary of a package that depends on `subetha`, as
[Installation](../installation/) sets up. The C program builds against
the prefix [SubEtha from C and C++](../c-and-cpp/) installs.

## What happened

The map lives in `observed.map` and stays after the program ends. The
counts do not: they belong to the registration, which lives in the
process that made it. Each record goes into a ring held in that
process's memory beside its handle on the map, so the sidecar counts
only what the process did through that handle. Another process working
the same map registers its own handle and reads its own counts.

The sidecar drains every registered structure on its own at least every
200 microseconds, so a program that reads the counts now and then needs
no scan of its own. The program above scans so the counts it prints
include every operation it made.

## What to do next

That closes the tutorial.

- The [sidecar reference](../../reference/subetha-sidecar/) lists every
  count and how a policy reads them.
- The [How-To Guides](../../how-to/) answer task-shaped questions.
- The [architecture overview](../../explanation/architecture/) explains
  how the structures, the observations and the sidecar fit together.
