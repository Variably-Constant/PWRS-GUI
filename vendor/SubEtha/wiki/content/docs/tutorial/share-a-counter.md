---
title: "Share a counter"
weight: 32
---

An atomic is one number in a file every process maps, read and changed
in single steps that no other process can get between. Two processes
adding to it lose no addition and need no lock. This page counts from
two processes, hands out a fixed number of tickets with
compare-and-exchange, publishes a result behind a flag, and keeps a word
of bits at 32 bits. Save the programs for your surface in one folder and
run them from it.

## Two processes, one counter

Each program adds one to the same counter a thousand times. `fetch_add`
reads, adds and writes back in one step, so however the two programs
interleave, no addition is lost and the total is exactly 2000. The
second program looks at the total until the first has finished too.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/first.rs" alongside="true"}
use std::sync::atomic::Ordering;

use subetha::cxc::SharedAtomicU64;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Whichever program arrives first creates the counter at zero, and the
    // other attaches to it, keeping whatever it already holds.
    let counter = SharedAtomicU64::create("counter.bin", 0)?;
    for _ in 0..1000 {
        counter.fetch_add(1, Ordering::SeqCst);
    }
    println!("first: added 1000");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="first.py" alongside="true"}
import subetha

# Whichever program arrives first creates the counter at zero, and the
# other attaches to it, keeping whatever it already holds.
counter = subetha.Atomic("counter.bin")
for _ in range(1000):
    counter.fetch_add(1)
print("first: added 1000")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="first.ps1" alongside="true"}
Import-Module SubEtha

# Whichever program arrives first creates the counter at zero, and the
# other attaches to it, keeping whatever it already holds.
$counter = New-SubEthaAtomic -Path counter.bin
foreach ($i in 1..1000) {
    $null = $counter.FetchAdd()
}
$counter.Dispose()
'first: added 1000'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="first.c" alongside="true"}
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

    /* Whichever program arrives first creates the counter at zero, and the
       other attaches to it, keeping whatever it already holds. */
    subetha_handle counter;
    if ((rc = subetha_atomic_u64_create("counter.bin", 0, SUBETHA_MODE_DEFAULT, &counter)) != SUBETHA_OK)
        return fail("counter", rc);
    for (int i = 0; i < 1000; i++) {
        if ((rc = subetha_atomic_u64_fetch_add(counter, 1, NULL)) != SUBETHA_OK) return fail("add", rc);
    }
    if ((rc = subetha_handle_destroy(counter)) != SUBETHA_OK) return fail("destroy", rc);
    printf("first: added 1000\n");
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
first: added 1000
```

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/second.rs"}
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use subetha::cxc::SharedAtomicU64;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let counter = SharedAtomicU64::create("counter.bin", 0)?;
    for _ in 0..1000 {
        counter.fetch_add(1, Ordering::SeqCst);
    }
    // The first program may still be adding: look at the total every
    // millisecond until it reaches 2000, for ten seconds at most.
    let deadline = Instant::now() + Duration::from_secs(10);
    while counter.load(Ordering::SeqCst) < 2000 {
        if Instant::now() > deadline {
            return Err("the total never reached 2000".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    println!("second: added 1000, total = {}", counter.load(Ordering::SeqCst));
    // Both programs have finished adding, so the counter goes back to zero
    // for the next run.
    counter.store(0, Ordering::SeqCst);
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="second.py"}
import time

import subetha

counter = subetha.Atomic("counter.bin")
for _ in range(1000):
    counter.fetch_add(1)

# The first program may still be adding: look at the total every
# millisecond until it reaches 2000, for ten seconds at most.
deadline = time.monotonic() + 10
while counter.load() < 2000:
    if time.monotonic() > deadline:
        raise RuntimeError("the total never reached 2000")
    time.sleep(0.001)
print(f"second: added 1000, total = {counter.load()}")

# Both programs have finished adding, so the counter goes back to zero for
# the next run.
counter.store(0)
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="second.ps1"}
Import-Module SubEtha

$counter = New-SubEthaAtomic -Path counter.bin
foreach ($i in 1..1000) {
    $null = $counter.FetchAdd()
}

# The first program may still be adding: look at the total every
# millisecond until it reaches 2000, for ten seconds at most.
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ($counter.Load() -lt 2000) {
    if ([DateTime]::UtcNow -gt $deadline) { throw 'the total never reached 2000' }
    Start-Sleep -Milliseconds 1
}
"second: added 1000, total = $($counter.Load())"

# Both programs have finished adding, so the counter goes back to zero for
# the next run.
$counter.Store(0)
$counter.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="second.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>
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

/* Wait a millisecond before looking again. */
static void pause_a_millisecond(void) {
#if defined(_WIN32)
    Sleep(1);
#else
    struct timespec ts = {0, 1000000};
    nanosleep(&ts, NULL);
#endif
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle counter;
    if ((rc = subetha_atomic_u64_create("counter.bin", 0, SUBETHA_MODE_DEFAULT, &counter)) != SUBETHA_OK)
        return fail("counter", rc);
    for (int i = 0; i < 1000; i++) {
        if ((rc = subetha_atomic_u64_fetch_add(counter, 1, NULL)) != SUBETHA_OK) return fail("add", rc);
    }

    /* The first program may still be adding: look at the total every
       millisecond until it reaches 2000, for ten seconds at most. */
    time_t deadline = time(NULL) + 10;
    uint64_t total;
    for (;;) {
        if ((rc = subetha_atomic_u64_load(counter, &total)) != SUBETHA_OK) return fail("load", rc);
        if (total >= 2000) break;
        if (time(NULL) > deadline) {
            fprintf(stderr, "the total never reached 2000\n");
            return 1;
        }
        pause_a_millisecond();
    }
    printf("second: added 1000, total = %llu\n", (unsigned long long)total);

    /* Both programs have finished adding, so the counter goes back to zero
       for the next run. */
    if ((rc = subetha_atomic_u64_store(counter, 0)) != SUBETHA_OK) return fail("store", rc);
    if ((rc = subetha_handle_destroy(counter)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
second: added 1000, total = 2000
```

Start the first and, while it runs, the second, from the folder holding
both. The `sh` lines are for a POSIX shell and the `PS>` lines for
PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin first & cargo run --bin second; wait
```
```powershell
PS> $first = Start-Process cargo -ArgumentList 'run', '--bin', 'first' -NoNewWindow -PassThru
PS> cargo run --bin second
PS> $first.WaitForExit()
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python first.py & python second.py; wait
```
```powershell
PS> $first = Start-Process python -ArgumentList 'first.py' -NoNewWindow -PassThru
PS> python second.py
PS> $first.WaitForExit()
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> $first = Start-Process pwsh -ArgumentList '-File', 'first.ps1' -NoNewWindow -PassThru
PS> pwsh -File second.ps1
PS> $first.WaitForExit()
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc first.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o first
cc second.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o second
./first & ./second; wait
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs, which also gives the CMake build and the Windows libraries.

`create` makes the counter, holding the value given, when the file does
not exist, and attaches to the live value when it does, so the two
programs can start in either order. The counter outlasts both
processes: the second puts it back to zero with `store` once the total
is in, so the pair can run again. A program that should sleep until
another process signals it, rather than look every millisecond, wants a
[condition variable](../../reference/subetha-cxc/coordination-types/shared-condvar/).

## At most five tickets

A count that must stop at a limit cannot use `fetch_add`, which adds
whatever the count already is. Compare-and-exchange puts a new value in
only if the value is still the one the caller read, and answers what it
found either way. A caller that gets back what it expected has made its
change; any other answer is what another process left there, and the
caller tries again from it. This program runs alone, though every
process that opens `tickets.bin` draws on the same five.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/tickets.rs"}
use std::sync::atomic::Ordering;

use subetha::cxc::SharedAtomicU64;

const LIMIT: u64 = 5;

/// The next ticket, or `None` once all five are out. Another process may
/// take one between the load and the exchange, so the exchange puts the
/// new count in only if the count is still the one this read, and hands
/// back the count it found when it is not.
fn take(issued: &SharedAtomicU64) -> Option<u64> {
    let mut seen = issued.load(Ordering::SeqCst);
    while seen < LIMIT {
        match issued.compare_exchange(seen, seen + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return Some(seen + 1),
            Err(found) => seen = found,
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // This program owns tickets.bin, so it remakes the file at zero,
    // whatever an earlier run left in it.
    let issued = SharedAtomicU64::reset("tickets.bin", 0)?;
    while let Some(ticket) = take(&issued) {
        println!("took ticket {ticket}");
    }
    println!("none left after {LIMIT}");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="tickets.py"}
import subetha

LIMIT = 5


def take(issued):
    """The next ticket, or None once all five are out. Another process may
    take one between the load and the exchange, so the exchange puts the
    new count in only if the count is still the one this read, and answers
    the count it found either way."""
    seen = issued.load()
    while seen < LIMIT:
        found = issued.compare_exchange(seen, seen + 1)
        if found == seen:
            return seen + 1
        seen = found
    return None


# This program owns tickets.bin, so it remakes the file at zero, whatever
# an earlier run left in it.
issued = subetha.Atomic.reset("tickets.bin", 0)
while (ticket := take(issued)) is not None:
    print(f"took ticket {ticket}")
print(f"none left after {LIMIT}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="tickets.ps1"}
Import-Module SubEtha

$limit = 5

# The next ticket, or $null once all five are out. Another process may take
# one between the load and the exchange, so the exchange puts the new count
# in only if the count is still the one this read, and answers the count it
# found either way.
function Get-Ticket($issued) {
    $seen = $issued.Load()
    while ($seen -lt $limit) {
        $found = $issued.CompareExchange($seen, $seen + 1)
        if ($found -eq $seen) { return $seen + 1 }
        $seen = $found
    }
    return $null
}

# This program owns tickets.bin, so it remakes the file at zero, whatever
# an earlier run left in it.
$issued = New-SubEthaAtomic -Path tickets.bin -Init 0 -Reset
while ($null -ne ($ticket = Get-Ticket $issued)) {
    "took ticket $ticket"
}
"none left after $limit"
$issued.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="tickets.c"}
#include "subetha.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#define LIMIT 5u

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

/* The next ticket into *ticket, or 0 once all five are out. Another
   process may take one between the load and the exchange, so the exchange
   puts the new count in only if the count is still the one this read, and
   hands back the count it found either way. */
static int32_t take(subetha_handle issued, uint64_t *ticket) {
    *ticket = 0;
    uint64_t seen;
    int32_t rc = subetha_atomic_u64_load(issued, &seen);
    if (rc != SUBETHA_OK) return rc;
    while (seen < LIMIT) {
        bool swapped;
        rc = subetha_atomic_u64_compare_exchange(issued, seen, seen + 1, &seen, &swapped);
        if (rc != SUBETHA_OK) return rc;
        if (swapped) {
            *ticket = seen + 1;
            return SUBETHA_OK;
        }
    }
    return SUBETHA_OK;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* This program owns tickets.bin, so it remakes the file at zero,
       whatever an earlier run left in it. */
    subetha_handle issued;
    if ((rc = subetha_atomic_u64_reset("tickets.bin", 0, SUBETHA_MODE_DEFAULT, &issued)) != SUBETHA_OK)
        return fail("reset", rc);
    for (;;) {
        uint64_t ticket;
        if ((rc = take(issued, &ticket)) != SUBETHA_OK) return fail("take", rc);
        if (ticket == 0) break;
        printf("took ticket %llu\n", (unsigned long long)ticket);
    }
    printf("none left after %u\n", LIMIT);
    if ((rc = subetha_handle_destroy(issued)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
took ticket 1
took ticket 2
took ticket 3
took ticket 4
took ticket 5
none left after 5
```

Run it on its own, from the same folder:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin tickets
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python tickets.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File tickets.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc tickets.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o tickets
./tickets
```
{{< /tab >}}

{{< /tabs >}}

`reset` remakes the file holding the value given, whatever it held
before, which is why each run starts from zero. Reset only a file no
other process is using: Windows refuses while any process still has it
open, and elsewhere the reset discards the value the other processes
see.

## Publish a result behind a flag

A flag can say when something another process wrote is ready to read.
The publisher writes its result and then raises the flag with release
ordering, and the reader lowers the flag with acquire ordering. A reader
whose acquire sees the flag raised also sees every write the publisher
made before its release, so it reads the result that was published and
never an older one. With relaxed ordering on both sides, nothing would
stop the reader seeing the flag before the result.

Start the reader first. It looks at the flag every millisecond until the
publisher raises it.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/reader.rs" alongside="true"}
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use subetha::cxc::{SharedAtomicBool, SharedAtomicU64};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The reader may start before the publisher: whichever arrives first
    // makes the flag, lowered, and the result, at zero.
    let ready = SharedAtomicBool::create("ready.bin", false)?;
    let result = SharedAtomicU64::create("result.bin", 0)?;
    // Swapping in false lowers the flag and answers whether it was raised,
    // in one step. Acquire pairs with the publisher's release: once this
    // sees the flag raised, it sees the result written before it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.swap(false, Ordering::Acquire) {
        if Instant::now() > deadline {
            return Err("nothing was published".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    println!("reader: the result is {}", result.load(Ordering::Relaxed));
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="reader.py" alongside="true"}
import time

import subetha

# The reader may start before the publisher: whichever arrives first makes
# the flag, lowered, and the result, at zero.
ready = subetha.AtomicBool("ready.bin")
result = subetha.Atomic("result.bin")

# Swapping in False lowers the flag and answers whether it was raised, in
# one step. Acquire pairs with the publisher's release: once this sees the
# flag raised, it sees the result written before it.
deadline = time.monotonic() + 10
while not ready.swap(False, "acquire"):
    if time.monotonic() > deadline:
        raise RuntimeError("nothing was published")
    time.sleep(0.001)
print(f"reader: the result is {result.load('relaxed')}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="reader.ps1" alongside="true"}
Import-Module SubEtha

# The reader may start before the publisher: whichever arrives first makes
# the flag, lowered, and the result, at zero.
$ready = New-SubEthaAtomicBool -Path ready.bin
$result = New-SubEthaAtomic -Path result.bin

# Swapping in $false lowers the flag and answers whether it was raised, in
# one step. Acquire pairs with the publisher's Release: once this sees the
# flag raised, it sees the result written before it.
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while (-not $ready.Swap($false, 'Acquire')) {
    if ([DateTime]::UtcNow -gt $deadline) { throw 'nothing was published' }
    Start-Sleep -Milliseconds 1
}
"reader: the result is $($result.Load('Relaxed'))"
$result.Dispose()
$ready.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="reader.c" alongside="true"}
#include "subetha.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
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

/* Wait a millisecond before looking again. */
static void pause_a_millisecond(void) {
#if defined(_WIN32)
    Sleep(1);
#else
    struct timespec ts = {0, 1000000};
    nanosleep(&ts, NULL);
#endif
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* The reader may start before the publisher: whichever arrives first
       makes the flag, lowered, and the result, at zero. */
    subetha_handle ready, result;
    if ((rc = subetha_atomic_bool_create("ready.bin", false, SUBETHA_MODE_DEFAULT, &ready)) != SUBETHA_OK)
        return fail("flag", rc);
    if ((rc = subetha_atomic_u64_create("result.bin", 0, SUBETHA_MODE_DEFAULT, &result)) != SUBETHA_OK)
        return fail("result", rc);

    /* Swapping in false lowers the flag and hands back whether it was
       raised, in one step. Acquire pairs with the publisher's release: once
       this sees the flag raised, it sees the result written before it. */
    time_t deadline = time(NULL) + 10;
    for (;;) {
        bool raised;
        if ((rc = subetha_atomic_bool_swap_explicit(ready, false, SUBETHA_ORDER_ACQUIRE, &raised)) != SUBETHA_OK)
            return fail("swap", rc);
        if (raised) break;
        if (time(NULL) > deadline) {
            fprintf(stderr, "nothing was published\n");
            return 1;
        }
        pause_a_millisecond();
    }
    uint64_t value;
    if ((rc = subetha_atomic_u64_load_explicit(result, SUBETHA_ORDER_RELAXED, &value)) != SUBETHA_OK)
        return fail("load", rc);

    subetha_handle handles[] = {result, ready};
    for (size_t i = 0; i < sizeof handles / sizeof handles[0]; i++) {
        if ((rc = subetha_handle_destroy(handles[i])) != SUBETHA_OK) return fail("destroy", rc);
    }
    printf("reader: the result is %llu\n", (unsigned long long)value);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
reader: the result is 5050
```

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/publisher.rs"}
use std::sync::atomic::Ordering;

use subetha::cxc::{SharedAtomicBool, SharedAtomicU64};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ready = SharedAtomicBool::create("ready.bin", false)?;
    let result = SharedAtomicU64::create("result.bin", 0)?;
    // Write the result, then raise the flag. Release makes every write
    // before it visible to a reader whose acquire sees the flag raised, so
    // the result needs no ordering of its own.
    let total: u64 = (1..=100).sum();
    result.store(total, Ordering::Relaxed);
    ready.store(true, Ordering::Release);
    println!("publisher: published {total}");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="publisher.py"}
import subetha

ready = subetha.AtomicBool("ready.bin")
result = subetha.Atomic("result.bin")

# Write the result, then raise the flag. Release makes every write before
# it visible to a reader whose acquire sees the flag raised, so the result
# needs no ordering of its own.
total = sum(range(1, 101))
result.store(total, "relaxed")
ready.store(True, "release")
print(f"publisher: published {total}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="publisher.ps1"}
Import-Module SubEtha

$ready = New-SubEthaAtomicBool -Path ready.bin
$result = New-SubEthaAtomic -Path result.bin

# Write the result, then raise the flag. Release makes every write before
# it visible to a reader whose acquire sees the flag raised, so the result
# needs no ordering of its own.
$total = [uint64](1..100 | Measure-Object -Sum).Sum
$result.Store($total, 'Relaxed')
$ready.Store($true, 'Release')
$result.Dispose()
$ready.Dispose()
"publisher: published $total"
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="publisher.c"}
#include "subetha.h"
#include <stdbool.h>
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

    subetha_handle ready, result;
    if ((rc = subetha_atomic_bool_create("ready.bin", false, SUBETHA_MODE_DEFAULT, &ready)) != SUBETHA_OK)
        return fail("flag", rc);
    if ((rc = subetha_atomic_u64_create("result.bin", 0, SUBETHA_MODE_DEFAULT, &result)) != SUBETHA_OK)
        return fail("result", rc);

    /* Write the result, then raise the flag. Release makes every write
       before it visible to a reader whose acquire sees the flag raised, so
       the result needs no ordering of its own. */
    uint64_t total = 0;
    for (uint64_t i = 1; i <= 100; i++) total += i;
    if ((rc = subetha_atomic_u64_store_explicit(result, total, SUBETHA_ORDER_RELAXED)) != SUBETHA_OK)
        return fail("store", rc);
    if ((rc = subetha_atomic_bool_store_explicit(ready, true, SUBETHA_ORDER_RELEASE)) != SUBETHA_OK)
        return fail("raise", rc);

    subetha_handle handles[] = {result, ready};
    for (size_t i = 0; i < sizeof handles / sizeof handles[0]; i++) {
        if ((rc = subetha_handle_destroy(handles[i])) != SUBETHA_OK) return fail("destroy", rc);
    }
    printf("publisher: published %llu\n", (unsigned long long)total);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
publisher: published 5050
```

Start the reader and, while it waits, the publisher:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin reader & cargo run --bin publisher; wait
```
```powershell
PS> $reader = Start-Process cargo -ArgumentList 'run', '--bin', 'reader' -NoNewWindow -PassThru
PS> cargo run --bin publisher
PS> $reader.WaitForExit()
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python reader.py & python publisher.py; wait
```
```powershell
PS> $reader = Start-Process python -ArgumentList 'reader.py' -NoNewWindow -PassThru
PS> python publisher.py
PS> $reader.WaitForExit()
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> $reader = Start-Process pwsh -ArgumentList '-File', 'reader.ps1' -NoNewWindow -PassThru
PS> pwsh -File publisher.ps1
PS> $reader.WaitForExit()
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc reader.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o reader
cc publisher.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o publisher
./reader & ./publisher; wait
```
{{< /tab >}}

{{< /tabs >}}

Swapping in false lowers the flag and answers whether it was raised in
one step, so two readers can never both take the same publication, and
the flag is down again for the next run.

## A word of bits

A 32-bit atomic holds a counter or a set of bits that other code keeps
at 32 bits, such as a C `uint32_t`. Here each role owns one bit, so one
load tells which roles are present. `fetch_or` sets bits, `fetch_and`
with a bit's complement clears it, and `fetch_xor` flips it, and each
answers the word as it was before, which tells the caller whether its
bit was already set. This program runs alone.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/roles.rs"}
use std::sync::atomic::Ordering;

use subetha::cxc::SharedAtomicU32;

const WRITER: u32 = 1; // bit 0
const READER: u32 = 1 << 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // This program owns roles.bin, so it remakes the file at zero.
    let roles = SharedAtomicU32::reset("roles.bin", 0)?;
    // fetch_or sets bits and answers the word before, which says whether
    // another process had set them already.
    let before = roles.fetch_or(WRITER, Ordering::SeqCst);
    let was_set = (before & WRITER) != 0;
    println!("the writer's bit was set before: {}", if was_set { "yes" } else { "no" });
    roles.fetch_or(READER, Ordering::SeqCst);
    println!("roles: {:#x}", roles.load(Ordering::SeqCst));
    // fetch_and with a bit's complement clears it, and fetch_xor flips it.
    roles.fetch_and(!WRITER, Ordering::SeqCst);
    println!("after the writer clears its bit: {:#x}", roles.load(Ordering::SeqCst));
    roles.fetch_xor(READER, Ordering::SeqCst);
    println!("after the reader flips its bit: {:#x}", roles.load(Ordering::SeqCst));
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="roles.py"}
import subetha

WRITER = 1  # bit 0
READER = 1 << 3

# This program owns roles.bin, so it remakes the file at zero.
roles = subetha.AtomicU32.reset("roles.bin", 0)
# fetch_or sets bits and answers the word before, which says whether
# another process had set them already.
before = roles.fetch_or(WRITER)
print("the writer's bit was set before:", "yes" if before & WRITER else "no")
roles.fetch_or(READER)
print(f"roles: {roles.load():#x}")
# fetch_and with a bit's complement clears it, and fetch_xor flips it. A
# complement in Python is negative, so it is cut to the atomic's 32 bits.
roles.fetch_and(~WRITER & 0xFFFFFFFF)
print(f"after the writer clears its bit: {roles.load():#x}")
roles.fetch_xor(READER)
print(f"after the reader flips its bit: {roles.load():#x}")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="roles.ps1"}
Import-Module SubEtha

$writer = [uint32]1  # bit 0
$reader = [uint32]8  # bit 3

# This program owns roles.bin, so it remakes the file at zero.
$roles = New-SubEthaAtomicU32 -Path roles.bin -Init 0 -Reset
# FetchOr sets bits and answers the word before, which says whether
# another process had set them already.
$before = $roles.FetchOr($writer)
"the writer's bit was set before: $(if ($before -band $writer) { 'yes' } else { 'no' })"
$null = $roles.FetchOr($reader)
'roles: 0x{0:x}' -f $roles.Load()
# FetchAnd with a bit's complement clears it, and FetchXor flips it.
$null = $roles.FetchAnd(-bnot $writer)
'after the writer clears its bit: 0x{0:x}' -f $roles.Load()
$null = $roles.FetchXor($reader)
'after the reader flips its bit: 0x{0:x}' -f $roles.Load()
$roles.Dispose()
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="roles.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>

#define WRITER ((uint32_t)1) /* bit 0 */
#define READER ((uint32_t)1 << 3)

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

/* Print the word as it is now, after `what`. */
static int32_t show(subetha_handle roles, const char *what) {
    uint32_t word;
    int32_t rc = subetha_atomic_u32_load(roles, &word);
    if (rc == SUBETHA_OK) printf("%s: 0x%x\n", what, (unsigned)word);
    return rc;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* This program owns roles.bin, so it remakes the file at zero. */
    subetha_handle roles;
    if ((rc = subetha_atomic_u32_reset("roles.bin", 0, SUBETHA_MODE_DEFAULT, &roles)) != SUBETHA_OK)
        return fail("reset", rc);

    /* fetch_or sets bits and hands back the word before, which says
       whether another process had set them already. */
    uint32_t before;
    if ((rc = subetha_atomic_u32_fetch_or(roles, WRITER, &before)) != SUBETHA_OK) return fail("set", rc);
    printf("the writer's bit was set before: %s\n", (before & WRITER) != 0 ? "yes" : "no");
    if ((rc = subetha_atomic_u32_fetch_or(roles, READER, NULL)) != SUBETHA_OK) return fail("set", rc);
    if ((rc = show(roles, "roles")) != SUBETHA_OK) return fail("load", rc);

    /* fetch_and with a bit's complement clears it, and fetch_xor flips it. */
    if ((rc = subetha_atomic_u32_fetch_and(roles, ~WRITER, NULL)) != SUBETHA_OK) return fail("clear", rc);
    if ((rc = show(roles, "after the writer clears its bit")) != SUBETHA_OK) return fail("load", rc);
    if ((rc = subetha_atomic_u32_fetch_xor(roles, READER, NULL)) != SUBETHA_OK) return fail("flip", rc);
    if ((rc = show(roles, "after the reader flips its bit")) != SUBETHA_OK) return fail("load", rc);

    if ((rc = subetha_handle_destroy(roles)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
the writer's bit was set before: no
roles: 0x9
after the writer clears its bit: 0x8
after the reader flips its bit: 0x0
```

Run it on its own, from the same folder:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin roles
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python roles.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File roles.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc roles.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o roles
./roles
```
{{< /tab >}}

{{< /tabs >}}

## Widths and orderings

Each surface names the three widths this way:

| | 64 bits | 32 bits | Flag |
|---|---|---|---|
| Rust | `SharedAtomicU64` | `SharedAtomicU32` | `SharedAtomicBool` |
| Python | `Atomic` | `AtomicU32` | `AtomicBool` |
| PowerShell | `New-SubEthaAtomic` | `New-SubEthaAtomicU32` | `New-SubEthaAtomicBool` |
| C | `subetha_atomic_u64_*` | `subetha_atomic_u32_*` | `subetha_atomic_bool_*` |

A file records the width it was made at, and every process must open it
at that width: another is refused with the layout error that
[Read a failure](../read-a-failure/) shows how to read. The two integers
add, subtract, set, clear and flip bits, swap and compare-and-exchange;
the flag loads, stores and swaps. Arithmetic wraps at the width, so one
added to 4294967295 in a 32-bit atomic gives 0, on every surface,
Python's unbounded integers included.

Every operation takes a memory ordering, which says how it is ordered
against the other memory the same thread reads and writes. Leaving it
out means sequentially consistent, the strongest:

| Surface | Naming an ordering | When it is left out |
|---|---|---|
| Rust | `Ordering::Acquire`, on every call | it cannot be |
| Python | `"acquire"`, the last argument | `"seq_cst"` |
| PowerShell | `'Acquire'` or `[SubEtha.MemoryOrder]::Acquire` | `SeqCst` |
| C | `SUBETHA_ORDER_ACQUIRE`, to a call's `_explicit` form | the plain form is sequentially consistent |

A load takes relaxed, acquire or sequentially consistent, and a store
relaxed, release or sequentially consistent. Another ordering is
refused, with `ValueError` in Python, an argument error in PowerShell
and `SUBETHA_E_INVALID_ARGUMENT` in C, and Rust's standard library
panics on it. A read-modify-write, such as `fetch_add`, `swap` or
compare-and-exchange, takes any of the five. Compare-and-exchange's
read when the value does not match is acquire in Python and PowerShell;
Rust and C name it as a second ordering.

## What to do next

- [Claim a role across processes](../claim-a-role/) gives one job to one
  process at a time, and to another once that one stops.
- [Read a failure](../read-a-failure/) reads what each surface reports
  when a file holds something other than what was asked for.
- The [shared atomic reference](../../reference/subetha-cxc/atomics/shared-atomic/)
  gives the file's layout and every operation.
