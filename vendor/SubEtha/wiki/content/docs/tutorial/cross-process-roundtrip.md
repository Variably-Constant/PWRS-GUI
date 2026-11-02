---
title: "Cross-process round trip"
weight: 30
---

Two processes share a hash map through one file. A producer writes 100
entries and exits; a consumer started after it attaches to the same file
and reads them back. Save both programs for your surface in one folder
and run them from it, the producer first.

## The producer

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/producer.rs"}
use subetha::cxc::SharedHashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1024 slots of u32 keys and u64 values, created when the file does
    // not exist and attached to, entries and all, when it does.
    let map = SharedHashMap::<u32, u64>::create("roundtrip.bin", 1024)?;
    for k in 0..100u32 {
        map.insert(k, u64::from(k) * 1000)?;
    }
    println!("producer: 100 entries written to roundtrip.bin");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="producer.py"}
import subetha

# 1024 slots of 4-byte keys and 8-byte values, created when the file does
# not exist and attached to, entries and all, when it does.
with subetha.HashMap("roundtrip.bin", 1024, 4, 8) as m:
    for k in range(100):
        # insert answers "inserted" or "updated", and None when the map is full.
        if m.insert(k.to_bytes(4, "little"), (k * 1000).to_bytes(8, "little")) is None:
            raise RuntimeError("the map is full")

print("producer: 100 entries written to roundtrip.bin")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="producer.ps1"}
Import-Module SubEtha

# 1024 slots of 4-byte keys and 8-byte values, created when the file does
# not exist and attached to, entries and all, when it does.
$m = New-SubEthaHashMap -Path roundtrip.bin -Capacity 1024 -KeySize 4 -ValueSize 8
foreach ($k in 0..99) {
    $key = [BitConverter]::GetBytes([uint32]$k)
    $value = [BitConverter]::GetBytes([uint64]($k * 1000))
    # Insert answers Inserted, Updated, or Full.
    if ($m.Insert($key, $value) -eq 'Full') { throw 'the map is full' }
}
$m.Dispose()
'producer: 100 entries written to roundtrip.bin'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="producer.c"}
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

    /* 1024 slots of 4-byte keys and 8-byte values, created when the file
       does not exist and attached to, entries and all, when it does. */
    subetha_handle map;
    rc = subetha_hashmap_create("roundtrip.bin", 1024, 4, 8, SUBETHA_MODE_DEFAULT, &map);
    if (rc != SUBETHA_OK) return fail("create", rc);

    for (uint32_t k = 0; k < 100; k++) {
        uint64_t value = (uint64_t)k * 1000;
        rc = subetha_hashmap_insert(map, (const uint8_t *)&k, sizeof k,
                                    (const uint8_t *)&value, sizeof value, NULL);
        if (rc != SUBETHA_OK) return fail("insert", rc);
    }

    rc = subetha_handle_destroy(map);
    if (rc != SUBETHA_OK) return fail("destroy", rc);
    printf("producer: 100 entries written to roundtrip.bin\n");
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
producer: 100 entries written to roundtrip.bin
```

## The consumer

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/consumer.rs"}
use subetha::cxc::SharedHashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Attach to the producer's map. The capacity must be the one it was
    // created with; another is a LayoutMismatch.
    let map = SharedHashMap::<u32, u64>::open("roundtrip.bin", 1024)?;
    let mut sum = 0u64;
    for k in 0..100u32 {
        sum += map.get(&k).ok_or("an entry the producer wrote is missing")?;
    }
    println!("consumer: sum = {sum}");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="consumer.py"}
import subetha

# Attach to the producer's map. The capacity and both sizes must be the
# ones it was created with, or open raises OSError.
total = 0
with subetha.HashMap.open("roundtrip.bin", 1024, 4, 8) as m:
    for k in range(100):
        v = m.get(k.to_bytes(4, "little"))
        if v is None:
            raise RuntimeError("an entry the producer wrote is missing")
        total += int.from_bytes(v, "little")

print(f"consumer: sum = {total}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="consumer.ps1"}
Import-Module SubEtha

# Attach to the producer's map. The capacity and both sizes must be the
# ones it was created with.
$m = Open-SubEthaHashMap -Path roundtrip.bin -Capacity 1024 -KeySize 4 -ValueSize 8
$sum = [uint64]0
foreach ($k in 0..99) {
    $v = $m.Get([BitConverter]::GetBytes([uint32]$k))
    if ($null -eq $v) { throw 'an entry the producer wrote is missing' }
    $sum += [BitConverter]::ToUInt64($v, 0)
}
$m.Dispose()
"consumer: sum = $sum"
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="consumer.c"}
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

    /* Attach to the producer's map. The capacity and both sizes must be
       the ones it was created with. */
    subetha_handle map;
    rc = subetha_hashmap_open("roundtrip.bin", 1024, 4, 8, SUBETHA_MODE_DEFAULT, &map);
    if (rc != SUBETHA_OK) return fail("open", rc);

    uint64_t sum = 0;
    for (uint32_t k = 0; k < 100; k++) {
        uint64_t value;
        size_t len;
        rc = subetha_hashmap_get(map, (const uint8_t *)&k, sizeof k,
                                 (uint8_t *)&value, sizeof value, &len);
        if (rc != SUBETHA_OK) return fail("get", rc);
        sum += value;
    }

    rc = subetha_handle_destroy(map);
    if (rc != SUBETHA_OK) return fail("destroy", rc);
    printf("consumer: sum = %llu\n", (unsigned long long)sum);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
consumer: sum = 4950000
```

0 + 1000 + 2000 + ... + 99000 is 4,950,000.

## Running them

From the folder holding both programs, the producer to its end and then
the consumer:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin producer
cargo run --bin consumer
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python producer.py
python consumer.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File producer.ps1
PS> pwsh -File consumer.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc producer.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o producer
cc consumer.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o consumer
./producer
./consumer
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are two binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs, which also gives the CMake build and the Windows libraries.

## What happened

Both processes mapped `roundtrip.bin`, and the operating system backs
the two mappings with the same physical pages, so the consumer read the
bytes the producer's inserts wrote. Nothing was sent between them. The
file keeps the entries after both processes exit: run the consumer
again and it reads the same sum.

Every surface hashes keys with FNV-1a, which gives the same slot for the
same key in every process. Rust's default hasher seeds itself per
process, so it could not place a key where another process looks for it.

## What to do next

- [Reading observations](../reading-observations/) registers a structure
  with the sidecar and reads what it records.
- The [role-pair selection how-to](../../how-to/role-pair-selection/)
  picks a primitive for a concurrency shape.
- The [reference](../../reference/subetha-cxc/) lists every primitive
  with its layout and operations.
