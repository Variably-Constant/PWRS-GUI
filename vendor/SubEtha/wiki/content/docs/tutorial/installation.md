---
title: "Installation"
weight: 20
---

SubEtha has four surfaces: the Rust crates, the Python package
`subetha-ipc` (imported as `subetha`), the PowerShell module
`SubEtha`, and the C library `subetha-ffi`. The Python and PowerShell
surfaces ship prebuilt; the Rust crates and the C library build with
a Rust toolchain.

The Rust crates build on **stable Rust 1.96+** with the 2024 edition
(`rust-version = "1.96"` in the workspace manifest). No
nightly features. The CXC stack, the substrate, the sidecar, and
the pointer kit all compile with the stock stable toolchain.

The repo pins this in `rust-toolchain.toml` so `rustup` picks the
right channel on first build. You do not need to switch toolchains
by hand.

```toml
# rust-toolchain.toml (already in the repo)
[toolchain]
channel = "stable"
components = ["rustc", "cargo", "rustfmt", "clippy"]
profile = "default"
```

Windows, Linux, macOS, and FreeBSD all work; pick a C linker
through `rustup` (MSVC on Windows, clang or gcc elsewhere; FreeBSD
ships clang in base). The full test suite runs on Windows, Linux,
and FreeBSD as part of the project's own verification.

## Prerequisites

{{< tabs >}}

{{< tab name="Rust" >}}
A stable Rust toolchain via [rustup](https://rustup.rs/). A C
linker. That is the whole list.
{{< /tab >}}

{{< tab name="Python" >}}
CPython 3.11 or later. One wheel serves 3.11 and every later version
through the stable ABI; a free-threaded interpreter takes a wheel of
its own (see [SubEtha from Python](../python/)).
{{< /tab >}}

{{< tab name="PowerShell" >}}
PowerShell 7 or Windows PowerShell 5.1: one module folder serves
both.
{{< /tab >}}

{{< tab name="C" >}}
A C compiler, and a stable Rust toolchain via
[rustup](https://rustup.rs/) to build the library once.
{{< /tab >}}

{{< /tabs >}}

## Add SubEtha to your project

{{< tabs >}}

{{< tab name="Rust" >}}
```toml
[dependencies]
subetha = "0.6"
```

`subetha` re-exports the rest of the stack as `subetha::cxc`,
`subetha::pointers`, `subetha::sidecar` and `subetha::core`. Its
features `quic-bridge`, `tcp-bridge`, `tcp-tls-bridge`, `tls`,
`wire-locale` and `linux-futex-raw` turn on the same features of
`subetha-cxc`.
{{< /tab >}}

{{< tab name="Python" >}}
```sh
pip install subetha-ipc
```

The package imports as `subetha`.
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> Install-Module SubEtha
PS> Import-Module SubEtha
```
{{< /tab >}}

{{< tab name="C" >}}
From a clone of the repository:

```sh
cargo run -p xtask -- ffi-install --prefix /opt/subetha
```

The prefix holds `include/subetha.h`, the shared and static
libraries, a pkg-config file and a CMake package.
[SubEtha from C and C++](../c-and-cpp/) links a program against them.
{{< /tab >}}

{{< /tabs >}}

The Rust crate inventory:

| Crate | What's in it |
|---|---|
| `subetha` | The crate to depend on. Re-exports the four below as `subetha::cxc`, `subetha::pointers`, `subetha::sidecar` and `subetha::core`. |
| `subetha-cxc` | `Channel<T>`, `AdaptiveIpc<T>`, `AutoIpc`, the MMF dispatcher, and more than sixty MMF-backed primitives. |
| `subetha-pointers` | Exotic pointer types in eight families: `UmbraPointer`, `BloomPointer`, `CardinalityPointer`, `KStepPointer`, `KTower2/3`, `SelfDescPointer`, `VersionedPointer` / `HlcVersionedPointer` / `VectorClockPointer`, `ReadableCapability` / `WritableCapability`. |
| `subetha-core` | Handshake header, observation ring, migration protocol, `Marshal` trait, axis-signature catalog, CPUID helpers. The substrate. |
| `subetha-sidecar` | Registry, `AdaptiveInstance`, `Policy`, `SidecarBox`. The control plane. |
| `subetha-ffi` | The C ABI: generation-checked handles over the memory-mapped primitives, for C, C++ and every language that binds through C. See [SubEtha from C and C++](../c-and-cpp/). |

Depend on `subetha` alone; it brings the other four. A project that
wants one part of the stack and nothing else can depend on that
member crate directly.

## Your project also needs the same toolchain

The stable + edition-2024 requirement propagates. Add a
`rust-toolchain.toml` at the root of your downstream project so
`rustup` picks the same channel:

```toml
[toolchain]
channel = "stable"
```

You can also leave it off and rely on the workspace default, but
pinning makes builds reproducible across machines.

## Smoke test

The program creates a shared counter in a file, adds one to it, reads
it back, and removes the file again.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/main.rs"}
use std::sync::atomic::Ordering;

use subetha::cxc::SharedAtomicU64;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = SharedAtomicU64::create("smoke.bin", 0)?;
    a.fetch_add(1, Ordering::AcqRel);
    println!("value = {}", a.load(Ordering::Acquire));
    // The mapping goes with the handle, and then the file can go too.
    drop(a);
    std::fs::remove_file("smoke.bin")?;
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="smoke.py"}
import os

import subetha

a = subetha.Atomic("smoke.bin", 0)
a.fetch_add(1)
print(f"value = {a.load()}")
# The mapping goes with the last reference to it, and then the file can
# go too.
del a
os.remove("smoke.bin")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="smoke.ps1"}
Import-Module SubEtha

$a = New-SubEthaAtomic -Path smoke.bin -Init 0
$null = $a.FetchAdd(1)
"value = $($a.Load())"
# The mapping goes with Dispose, and then the file can go too.
$a.Dispose()
Remove-Item smoke.bin
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="smoke.c"}
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

    subetha_handle a;
    if ((rc = subetha_atomic_u64_create("smoke.bin", 0, SUBETHA_MODE_DEFAULT, &a)) != SUBETHA_OK)
        return fail("create", rc);
    uint64_t before, value;
    if ((rc = subetha_atomic_u64_fetch_add(a, 1, &before)) != SUBETHA_OK) return fail("add", rc);
    if ((rc = subetha_atomic_u64_load(a, &value)) != SUBETHA_OK) return fail("load", rc);
    printf("value = %llu\n", (unsigned long long)value);

    /* The mapping goes with the handle, and then the file can go too. */
    if ((rc = subetha_handle_destroy(a)) != SUBETHA_OK) return fail("destroy", rc);
    if (remove("smoke.bin") != 0) return 1;
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
value = 1
```

Run it from an empty folder:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --release
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python smoke.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File smoke.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc smoke.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o smoke
./smoke
```
{{< /tab >}}

{{< /tabs >}}

That confirms the MMF substrate maps and aliases correctly on your
host.

## Run the substrate microbench

The substrate has a fixed per-op cost floor. The `async_overhead`
bench checks your host matches it with a single-threaded `Channel<u64>`
round-trip (send + recv):

```sh
git clone https://github.com/Variably-Constant/SubEtha.git
cd SubEtha
cargo bench -p subetha-cxc --bench async_overhead
```

Each `b.iter` drives a 1000-round-trip inner loop, so every Criterion
`time:` line is the cost of 1000 round-trips - divide by 1000 for the
per-op figure. Reference per-round-trip numbers from an AMD Ryzen 9 7900X
(12-core, 24-thread) under Windows 11:

```text
sync     send/recv      ~ 4.5 ns/round-trip   (4.5 us per 1000-op batch)
blocking send/recv      ~  15 ns/round-trip
async    send/recv      ~ 125 ns/round-trip
```

If the sync figure is dramatically slower than ~4.5 ns, the most likely
cause is a debug build, not release - `cargo bench` always builds the
bench profile in release.

The bench runs from the Rust workspace. What a call costs from Python
and from PowerShell is in [SubEtha from Python](../python/) and
[SubEtha from PowerShell](../powershell/).

Now go to [Cross-process round trip](../cross-process-roundtrip/).
