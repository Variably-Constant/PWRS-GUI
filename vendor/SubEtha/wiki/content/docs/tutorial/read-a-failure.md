---
title: "Read a failure"
weight: 37
---

Every surface reports a failure in the form it uses for any failure: a
Rust call returns the structure's error, Python raises an exception,
PowerShell writes an error record or throws from a method, and a C call
returns a code and keeps a detail beside it. This page makes the same
mistakes on each surface, attaching to a map built at another size,
attaching to one that is not there, and inserting a key of the wrong
size, and reads what comes back. It ends with the answers that look like
failures and are not, and with what a panic looks like. Save each
surface's programs in one folder and run them from it in the order the
page gives them.

## A file of another shape

The program makes a map of 16 entries with 4-byte keys and 8-byte
values, then attaches to it asking for 8 entries. The file holds 16, so
the attach is refused, and the refusal names the field that differs,
what the file holds and what the call asked for.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/shape.rs"}
use subetha::cxc::{MapError, RawHashMap};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A map of 16 entries, 4-byte keys and 8-byte values.
    let _made = RawHashMap::create("map.bin", 16, 4, 8)?;
    // Attach asking for 8 entries.
    match RawHashMap::open("map.bin", 8, 4, 8) {
        Err(MapError::LayoutMismatch(m)) => {
            println!("refused: {m}");
            println!("field: {}, found: {}, wanted: {}", m.field, m.found, m.wanted);
        }
        Err(other) => return Err(other.into()),
        Ok(_) => return Err("a map of 16 entries attached as one of 8".into()),
    }
    Ok(())
}
```
```text {filename="Output"}
refused: the file's capacity is 16 where 8 was asked for
field: capacity, found: 16, wanted: 8
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="shape.py"}
import subetha

# A map of 16 entries, 4-byte keys and 8-byte values.
made = subetha.HashMap("map.bin", 16, 4, 8)
# Attach asking for 8 entries.
try:
    subetha.HashMap.open("map.bin", 8, 4, 8)
    raise RuntimeError("a map of 16 entries attached as one of 8")
except subetha.LayoutMismatch as e:
    print("refused:", e)
    print("an OSError:", isinstance(e, OSError))
```
```text {filename="Output"}
refused: attaching to the map: a map of another shape: the file's capacity is 16 where 8 was asked for: 'map.bin'
an OSError: True
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="shape.ps1"}
Import-Module SubEtha

# A map of 16 entries, 4-byte keys and 8-byte values.
$made = New-SubEthaHashMap -Path map.bin -Capacity 16 -KeySize 4 -ValueSize 8
# Attach asking for 8 entries. A cmdlet's failure is a record written to the
# error stream; -ErrorAction Stop makes it one that catch sees.
try {
    $null = Open-SubEthaHashMap -Path map.bin -Capacity 8 -KeySize 4 -ValueSize 8 -ErrorAction Stop
} catch {
    "category: $($_.CategoryInfo.Category)"
    # The message names the full path, then what was refused.
    "refused: $(($_.Exception.Message -split ': ', 2)[1])"
}
$made.Dispose()
```
```text {filename="Output"}
category: OpenError
refused: a map of another shape: the file's capacity is 16 where 8 was asked for
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="shape.c"}
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

    /* A map of 16 entries, 4-byte keys and 8-byte values. */
    subetha_handle made, attached;
    if ((rc = subetha_hashmap_create("map.bin", 16, 4, 8, SUBETHA_MODE_DEFAULT, &made)) != SUBETHA_OK)
        return fail("create", rc);

    /* Attach asking for 8 entries. The code names the kind of failure and
       the detail this one. */
    rc = subetha_hashmap_open("map.bin", 8, 4, 8, SUBETHA_MODE_DEFAULT, &attached);
    if (rc != SUBETHA_E_LAYOUT_MISMATCH) return fail("open", rc);
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    printf("refused: %s\n", subetha_strerror(rc));
    printf("detail: %s\n", detail);

    if ((rc = subetha_handle_destroy(made)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
```text {filename="Output"}
refused: a backing exists with a different layout
detail: 'map.bin': a map of another shape: the file's capacity is 16 where 8 was asked for
```
{{< /tab >}}

{{< /tabs >}}

Every structure refuses a file of another shape this way: the
structure's `LayoutMismatch` in Rust, carrying a
`subetha::cxc::layout::Mismatch` whose `field`, `found` and `wanted` a
program can read; `subetha.LayoutMismatch` in Python, an `OSError`, so
`except OSError` catches it along with every other failure to open a
file; an `OpenError` record in PowerShell; and `SUBETHA_E_LAYOUT_MISMATCH`
in C, whose detail starts with the path the call was given. A file
shorter than the layout reads "the file is 4096 bytes where 8192 were
asked for", and another structure's magic number prints in hexadecimal.

Without `-ErrorAction Stop` the PowerShell record goes to the error
stream and the script carries on, so `try` sees nothing; add
`-ErrorVariable failed` to collect records in `$failed` without
stopping.

## A file that is not there

An attach finds a structure or fails; it never makes an empty one. Here
the operating system refuses the open, and each surface carries the
system's answer.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/absent.rs"}
use std::error::Error;

use subetha::cxc::{MapError, RawHashMap};

fn main() -> Result<(), Box<dyn Error>> {
    match RawHashMap::open("absent.bin", 16, 4, 8) {
        // The map's error wraps the operating system's and hands it on from
        // source(): its kind reads the same on every system, and its code
        // and message are the system's.
        Err(refused @ MapError::IoError(_)) => {
            let io = refused
                .source()
                .and_then(|cause| cause.downcast_ref::<std::io::Error>())
                .ok_or("the map's error wraps no io::Error")?;
            println!("refused: {:?}", io.kind());
            println!("the system's code: {}", if io.raw_os_error().is_some() { "present" } else { "absent" });
        }
        Err(other) => return Err(other.into()),
        Ok(_) => return Err("a file that is not there was attached".into()),
    }
    Ok(())
}
```
```text {filename="Output"}
refused: NotFound
the system's code: present
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="absent.py"}
import errno

import subetha

try:
    subetha.HashMap.open("absent.bin", 16, 4, 8)
    raise RuntimeError("a file that is not there was attached")
except FileNotFoundError as e:
    # An OSError carrying the system's code, so it is a FileNotFoundError on
    # every system, with the path the call was given.
    print("refused:", type(e).__name__)
    print("errno is ENOENT:", e.errno == errno.ENOENT)
    print("filename:", e.filename)
```
```text {filename="Output"}
refused: FileNotFoundError
errno is ENOENT: True
filename: absent.bin
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="absent.ps1"}
Import-Module SubEtha

try {
    $null = Open-SubEthaHashMap -Path absent.bin -Capacity 16 -KeySize 4 -ValueSize 8 -ErrorAction Stop
} catch {
    # The category and the id say what kind of failure this is; the message
    # ends in the system's own text.
    "category: $($_.CategoryInfo.Category)"
    "id: $($_.FullyQualifiedErrorId)"
}
```
```text {filename="Output"}
category: OpenError
id: SubEthaOpen,Pwrs.Modules.SubEtha.OpenSubEthaHashMapCommand
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="absent.c"}
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

    subetha_handle map;
    rc = subetha_hashmap_open("absent.bin", 16, 4, 8, SUBETHA_MODE_DEFAULT, &map);
    if (rc != SUBETHA_E_IO) return fail("open", rc);
    /* The detail is the path, then the system's own message. */
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    printf("refused: %s\n", subetha_strerror(rc));
    printf("names the path: %s\n", strncmp(detail, "'absent.bin': ", 14) == 0 ? "yes" : "no");
    return subetha_shutdown();
}
```
```text {filename="Output"}
refused: the OS refused an open, map or remove
names the path: yes
```
{{< /tab >}}

{{< /tabs >}}

The system's message is its own wording, which differs between Windows
and Linux, so a program decides on the kind, the class or the code, and
shows the message to a person. In Rust the structure's error holds the
`std::io::Error` and returns it from `source()`, as every SubEtha error
that wraps another does, so a reporter that walks the chain prints
SubEtha's message and then the system's; the `std::io::Error` gives
`kind()` and `raw_os_error()`. Python's exception is the `OSError`
subclass the code names, `FileNotFoundError` and `PermissionError` among
them, with `errno`, `filename` and on Windows `winerror`; the PowerShell
record's id is the module's error id and the cmdlet that wrote it, and
its message ends in the system's text; and a C detail gives the path and
then the system's message, under `SUBETHA_E_IO`.

## A key of the wrong size

An argument the structure cannot take is refused before anything is
written. This map's keys are 4 bytes and the program offers 3.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/key.rs"}
use subetha::cxc::{MapError, RawHashMap};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let map = RawHashMap::create("map.bin", 16, 4, 8)?;
    // A 3-byte key in a map of 4-byte keys.
    match map.insert(&[1, 2, 3], &[0; 8]) {
        Err(e @ MapError::PayloadTooLarge) => println!("refused: {e}"),
        Err(other) => return Err(other.into()),
        Ok(_) => return Err("a 3-byte key was stored".into()),
    }
    Ok(())
}
```
```text {filename="Output"}
refused: the key and value do not fit a slot, or a slice is the wrong size
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="key.py"}
import subetha

m = subetha.HashMap("map.bin", 16, 4, 8)
# A 3-byte key in a map of 4-byte keys.
try:
    m.insert(b"abc", bytes(8))
    raise RuntimeError("a 3-byte key was stored")
except ValueError as e:
    print("refused:", e)
    print("an OSError:", isinstance(e, OSError))
```
```text {filename="Output"}
refused: inserting: the key and value do not fit a slot, or a slice is the wrong size
an OSError: False
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="key.ps1"}
Import-Module SubEtha

$map = New-SubEthaHashMap -Path map.bin -Capacity 16 -KeySize 4 -ValueSize 8
# A 3-byte key in a map of 4-byte keys. A method that fails throws, so catch
# sees it without -ErrorAction.
try {
    $null = $map.Insert([byte[]](1, 2, 3), [byte[]]::new(8))
} catch {
    "thrown: $($_.Exception.GetType().Name)"
    "refused: $($_.Exception.InnerException.Message)"
}
$map.Dispose()
```
```text {filename="Output"}
thrown: MethodInvocationException
refused: [SubEthaOperation] inserting: the key and value do not fit a slot, or a slice is the wrong size
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="key.c"}
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

    subetha_handle map;
    if ((rc = subetha_hashmap_create("map.bin", 16, 4, 8, SUBETHA_MODE_DEFAULT, &map)) != SUBETHA_OK)
        return fail("create", rc);
    /* A 3-byte key in a map of 4-byte keys. */
    const uint8_t key[3] = {1, 2, 3};
    const uint8_t value[8] = {0};
    rc = subetha_hashmap_insert(map, key, sizeof key, value, sizeof value, NULL);
    if (rc != SUBETHA_E_INVALID_ARGUMENT) return fail("insert", rc);
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    printf("refused: %s\n", subetha_strerror(rc));
    printf("detail: %s\n", detail);

    if ((rc = subetha_handle_destroy(map)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
```text {filename="Output"}
refused: an argument is null, zero, or out of range
detail: a key is 4 bytes, not 3
```
{{< /tab >}}

{{< /tabs >}}

Python raises `ValueError` for an argument, which is not an `OSError`,
so a handler for file failures does not swallow a programming mistake.
An index past the end raises `IndexError`, a key that has no entry
where one is required `KeyError`, a blocking wait that ran out
`TimeoutError`, and a write to a structure opened read-only
`io.UnsupportedOperation`. In PowerShell a method that fails throws a
`MethodInvocationException` whose `InnerException` is the module's own
error, its message led by the error's id in brackets, while a cmdlet
writes a record; C returns `SUBETHA_E_INVALID_ARGUMENT` with the detail.

## Answers that are not failures

An empty structure, a full one and an absent key are outcomes a program
plans for, and they come back as values rather than as errors wherever
the surface allows. Looking up a key the map does not hold:

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/lookup.rs"}
use subetha::cxc::RawHashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let map = RawHashMap::create("map.bin", 16, 4, 8)?;
    let mut value = [0u8; 8];
    // An absent key is an answer, not an error.
    let found = map.get(&7u32.to_le_bytes(), &mut value)?;
    println!("found: {found}");
    Ok(())
}
```
```text {filename="Output"}
found: false
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="lookup.py"}
import subetha

m = subetha.HashMap("map.bin", 16, 4, 8)
# An absent key is an answer, not an exception.
print("found:", m.get((7).to_bytes(4, "little")) is not None)
```
```text {filename="Output"}
found: False
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="lookup.ps1"}
Import-Module SubEtha

$map = New-SubEthaHashMap -Path map.bin -Capacity 16 -KeySize 4 -ValueSize 8
# An absent key is an answer, not an error.
"found: $($null -ne $map.Get([BitConverter]::GetBytes([uint32]7)))"
$map.Dispose()
```
```text {filename="Output"}
found: False
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="lookup.c"}
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

    subetha_handle map;
    if ((rc = subetha_hashmap_create("map.bin", 16, 4, 8, SUBETHA_MODE_DEFAULT, &map)) != SUBETHA_OK)
        return fail("create", rc);
    /* An absent key is a code of its own, not a failure of the call. */
    const uint8_t key[4] = {7, 0, 0, 0};
    uint8_t value[8];
    size_t len;
    rc = subetha_hashmap_get(map, key, sizeof key, value, sizeof value, &len);
    if (rc != SUBETHA_OK && rc != SUBETHA_E_MAP_KEY_ABSENT) return fail("get", rc);
    printf("found: %s\n", rc == SUBETHA_OK ? "true" : "false");

    if ((rc = subetha_handle_destroy(map)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
```text {filename="Output"}
found: false
```
{{< /tab >}}

{{< /tabs >}}

The same holds for a full map, which Rust reports as `MapError::Full`,
Python as an `insert` that answers `None`, PowerShell as an `Insert`
that answers `Full`, and C as `SUBETHA_E_MAP_FULL`; for an empty ring's
pop, which Python and PowerShell answer with `None` and `$null`; and for
a condition wait that ran out with its condition still false, which
`wait_for` in Python answers with `False` and `Wait-SubEthaCondition`
with `$false`. A C call that returns one of these outcome codes still
leaves a detail, the code's own name, so a program that prints every
detail it gets never prints an earlier call's.

## When SubEtha panics

A panic inside SubEtha is a bug, or a precondition its documentation
states that a call broke, such as a capacity below 2. In Rust it unwinds
through your call like any panic. Python raises
`pyo3_runtime.PanicException`, which derives from `BaseException`, so
`except Exception` does not catch it. PowerShell reports it as a
terminating error carrying the panic's message. A C call catches it at
the boundary and returns `SUBETHA_E_PANIC`; the object behind the
handle is then poisoned, and every later call on it returns
`SUBETHA_E_HANDLE_POISONED`. Two calls report on panics, and need no
handle:

```c {filename="panics.c"}
#include "subetha.h"
#include <stdint.h>
#include <stdio.h>

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return 1;

    /* How many calls have panicked in this process, and what the most
       recent panic said; zero bytes needed means there has been none. */
    printf("panics: %llu\n", (unsigned long long)subetha_panic_count());
    char message[256];
    size_t needed = subetha_last_panic_message(message, sizeof message);
    printf("last panic: %s\n", needed == 0 ? "none" : message);
    return subetha_shutdown();
}
```
```text {filename="Output"}
panics: 0
last panic: none
```

## Run them

Run each surface's programs from the folder that holds them, in the
order above. The `sh` lines are for a POSIX shell and the `PS>` lines
for PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
for p in shape absent key lookup; do cargo run --bin $p; done
```
```powershell
PS> foreach ($p in 'shape', 'absent', 'key', 'lookup') { cargo run --bin $p }
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
for p in shape absent key lookup; do python $p.py; done
```
```powershell
PS> foreach ($p in 'shape', 'absent', 'key', 'lookup') { python "$p.py" }
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> foreach ($p in 'shape', 'absent', 'key', 'lookup') { pwsh -File "$p.ps1" }
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
for p in shape absent key lookup panics; do
    cc $p.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o $p && ./$p
done
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs.

## What to do next

- [Reading observations](../reading-observations/) registers a structure
  with the sidecar and reads what it records.
- The [reference](../../reference/subetha-cxc/) lists every structure's
  errors in full.
