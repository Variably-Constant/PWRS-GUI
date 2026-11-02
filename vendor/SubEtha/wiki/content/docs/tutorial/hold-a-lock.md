---
title: "Claim a role across processes"
weight: 33
slug: claim-a-role
aliases:
  - /docs/tutorial/hold-a-lock/
---

Some jobs only one process may do at a time: writing one log, serving
one port, running one scheduler. A leader election hands that role to one
process. A claim returns at once, so no process ever waits on the one
holding the role; the holder keeps the role by beating, and once it stops,
because it exited or hung, another process takes the role over. Save both
programs for your surface in one folder and run them from it, the holder
first and then the standby.

## A process holds the role

The holder claims the role, beats a few times as a working holder does,
and then exits without stepping down, as a process that crashed would.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/holder.rs"}
use subetha::cxc::SharedLeaderElection;

/// This program's id in the election. A real program passes its process
/// id; a fixed one keeps the output the same on every run.
const ME: u32 = 1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Every process obtains the same election: whichever arrives first
    // creates it with nobody holding the role, and the rest attach.
    let election = SharedLeaderElection::create("role.bin")?;
    // A claim returns at once: true when this process now holds the role.
    if !election.try_claim_leadership(ME, 3) {
        return Err("the role was held".into());
    }
    println!("holder: holds the role, term {}", election.election_term());
    // A holder keeps the role by beating. The election counts time in
    // epochs, which any process may step.
    for _ in 0..3 {
        election.tick_epoch();
        election.beat_as_leader(ME);
    }
    // Leaving without step_down is what a crashed holder does.
    println!("holder: exits without stepping down");
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="holder.py"}
import subetha

# This program's id in the election. A real program passes none, and the
# election takes its process id; a fixed one keeps the output the same on
# every run.
ME = 1

# Every process obtains the same election: whichever arrives first
# creates it with nobody holding the role, and the rest attach.
election = subetha.LeaderElection("role.bin")
# A claim returns at once: True when this process now holds the role.
if not election.try_claim(ME, 3):
    raise RuntimeError("the role was held")
print(f"holder: holds the role, term {election.term}")
# A holder keeps the role by beating. The election counts time in epochs,
# which any process may step.
for _ in range(3):
    election.tick_epoch()
    election.beat(ME)
# Leaving without step_down is what a crashed holder does.
print("holder: exits without stepping down")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="holder.ps1"}
Import-Module SubEtha

# This program's id in the election. A real program passes none, and the
# election takes its process id; a fixed one keeps the output the same on
# every run.
$me = 1

# Every process obtains the same election: whichever arrives first
# creates it with nobody holding the role, and the rest attach.
$election = New-SubEthaLeaderElection -Path role.bin
# A claim returns at once: true when this process now holds the role.
if (-not $election.TryClaim(3, $me)) { throw 'the role was held' }
"holder: holds the role, term $($election.Term())"
# A holder keeps the role by beating. The election counts time in epochs,
# which any process may step.
foreach ($i in 1..3) {
    $null = $election.TickEpoch()
    $null = $election.Beat($me)
}
# Leaving without StepDown is what a crashed holder does.
'holder: exits without stepping down'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="holder.c"}
#include "subetha.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

/* This program's id in the election. A real program passes its process
   id, which subetha_current_pid gives; a fixed one keeps the output the
   same on every run. */
#define ME 1

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    /* Every process obtains the same election: whichever arrives first
       creates it with nobody holding the role, and the rest attach. */
    subetha_handle election;
    if ((rc = subetha_leader_create("role.bin", SUBETHA_MODE_DEFAULT, &election)) != SUBETHA_OK)
        return fail("election", rc);

    /* A claim returns at once: claimed is true when this process now
       holds the role. */
    bool claimed;
    if ((rc = subetha_leader_try_claim(election, ME, SUBETHA_LEADER_DEFAULT_GRACE, &claimed)) != SUBETHA_OK)
        return fail("claim", rc);
    if (!claimed) {
        fprintf(stderr, "the role was held\n");
        return 1;
    }
    struct subetha_leader_stats stats;
    if ((rc = subetha_leader_read_stats(election, &stats)) != SUBETHA_OK) return fail("stats", rc);
    printf("holder: holds the role, term %u\n", stats.election_term);

    /* A holder keeps the role by beating. The election counts time in
       epochs, which any process may step. */
    for (int i = 0; i < 3; i++) {
        if ((rc = subetha_leader_tick_epoch(election, NULL)) != SUBETHA_OK) return fail("tick", rc);
        if ((rc = subetha_leader_beat(election, ME, NULL)) != SUBETHA_OK) return fail("beat", rc);
    }

    /* Leaving without subetha_leader_step_down is what a crashed holder
       does. */
    printf("holder: exits without stepping down\n");
    if ((rc = subetha_handle_destroy(election)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
holder: holds the role, term 1
holder: exits without stepping down
```

## A standby takes it over

The standby claims the same role. Its first claim is refused at once,
because the holder's last beat is still fresh. It then steps the epoch
past the grace period, the three epochs a holder may go without beating,
and claims again: the holder counts as gone, and the role is the
standby's.

{{< tabs >}}

{{< tab name="Rust" >}}
```rust {filename="src/bin/standby.rs"}
use subetha::cxc::SharedLeaderElection;

const ME: u32 = 2;
/// Epochs a holder may go without beating before its role can be taken.
const GRACE: u64 = 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let election = SharedLeaderElection::create("role.bin")?;
    // The holder's last beat is fresh, so the claim is refused, and at
    // once: a claim never waits for the holder.
    if election.try_claim_leadership(ME, GRACE) {
        return Err("the role was taken from a holder whose beat is fresh".into());
    }
    println!("standby: refused while the holder's beat is fresh");
    // Step the epoch past the grace period: the holder, which no longer
    // beats, falls behind, and its role can be taken over.
    for _ in 0..=GRACE {
        election.tick_epoch();
    }
    if !election.try_claim_leadership(ME, GRACE) {
        return Err("the role of a holder that stopped beating was not taken over".into());
    }
    println!("standby: took the role over, term {}", election.election_term());
    // Stepping down hands the role on at once, with no grace to wait out.
    election.step_down(ME);
    match election.current_leader() {
        None => println!("standby: stepped down, nobody holds the role"),
        Some(id) => return Err(format!("{id} still holds the role").into()),
    }
    Ok(())
}
```
{{< /tab >}}

{{< tab name="Python" >}}
```python {filename="standby.py"}
import subetha

ME = 2
# Epochs a holder may go without beating before its role can be taken.
GRACE = 3

election = subetha.LeaderElection("role.bin")
# The holder's last beat is fresh, so the claim is refused, and at once:
# a claim never waits for the holder.
if election.try_claim(ME, GRACE):
    raise RuntimeError("the role was taken from a holder whose beat is fresh")
print("standby: refused while the holder's beat is fresh")
# Step the epoch past the grace period: the holder, which no longer beats,
# falls behind, and its role can be taken over.
for _ in range(GRACE + 1):
    election.tick_epoch()
if not election.try_claim(ME, GRACE):
    raise RuntimeError("the role of a holder that stopped beating was not taken over")
print(f"standby: took the role over, term {election.term}")
# Stepping down hands the role on at once, with no grace to wait out.
election.step_down(ME)
if election.leader is not None:
    raise RuntimeError(f"{election.leader} still holds the role")
print("standby: stepped down, nobody holds the role")
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell {filename="standby.ps1"}
Import-Module SubEtha

$me = 2
# Epochs a holder may go without beating before its role can be taken.
$grace = 3

$election = New-SubEthaLeaderElection -Path role.bin
# The holder's last beat is fresh, so the claim is refused, and at once:
# a claim never waits for the holder.
if ($election.TryClaim($grace, $me)) { throw 'the role was taken from a holder whose beat is fresh' }
"standby: refused while the holder's beat is fresh"
# Step the epoch past the grace period: the holder, which no longer beats,
# falls behind, and its role can be taken over.
foreach ($i in 0..$grace) { $null = $election.TickEpoch() }
if (-not $election.TryClaim($grace, $me)) { throw 'the role of a holder that stopped beating was not taken over' }
"standby: took the role over, term $($election.Term())"
# Stepping down hands the role on at once, with no grace to wait out.
$null = $election.StepDown($me)
if ($null -ne $election.Leader()) { throw "$($election.Leader()) still holds the role" }
'standby: stepped down, nobody holds the role'
```
{{< /tab >}}

{{< tab name="C" >}}
```c {filename="standby.c"}
#include "subetha.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#define ME 2
/* Epochs a holder may go without beating before its role can be taken. */
#define GRACE 3

static int fail(const char *what, int32_t rc) {
    char detail[256];
    subetha_last_error_detail(detail, sizeof detail);
    fprintf(stderr, "%s: %s (%s)\n", what, subetha_strerror(rc), detail);
    return 1;
}

int main(void) {
    int32_t rc = subetha_init(SUBETHA_MODE_STRICT);
    if (rc != SUBETHA_OK) return fail("init", rc);

    subetha_handle election;
    if ((rc = subetha_leader_create("role.bin", SUBETHA_MODE_DEFAULT, &election)) != SUBETHA_OK)
        return fail("election", rc);

    /* The holder's last beat is fresh, so the claim is refused, and at
       once: a claim never waits for the holder. */
    bool claimed;
    if ((rc = subetha_leader_try_claim(election, ME, GRACE, &claimed)) != SUBETHA_OK)
        return fail("claim", rc);
    if (claimed) {
        fprintf(stderr, "the role was taken from a holder whose beat is fresh\n");
        return 1;
    }
    printf("standby: refused while the holder's beat is fresh\n");

    /* Step the epoch past the grace period: the holder, which no longer
       beats, falls behind, and its role can be taken over. */
    for (int i = 0; i <= GRACE; i++) {
        if ((rc = subetha_leader_tick_epoch(election, NULL)) != SUBETHA_OK) return fail("tick", rc);
    }
    if ((rc = subetha_leader_try_claim(election, ME, GRACE, &claimed)) != SUBETHA_OK)
        return fail("claim", rc);
    if (!claimed) {
        fprintf(stderr, "the role of a holder that stopped beating was not taken over\n");
        return 1;
    }
    struct subetha_leader_stats stats;
    if ((rc = subetha_leader_read_stats(election, &stats)) != SUBETHA_OK) return fail("stats", rc);
    printf("standby: took the role over, term %u\n", stats.election_term);

    /* Stepping down hands the role on at once, with no grace to wait
       out. */
    if ((rc = subetha_leader_step_down(election, ME, NULL)) != SUBETHA_OK) return fail("step down", rc);
    if ((rc = subetha_leader_read_stats(election, &stats)) != SUBETHA_OK) return fail("stats", rc);
    if (stats.leader_pid != SUBETHA_LEADER_NONE) {
        fprintf(stderr, "%u still holds the role\n", stats.leader_pid);
        return 1;
    }
    printf("standby: stepped down, nobody holds the role\n");

    if ((rc = subetha_handle_destroy(election)) != SUBETHA_OK) return fail("destroy", rc);
    return subetha_shutdown();
}
```
{{< /tab >}}

{{< /tabs >}}

```text {filename="Output"}
standby: refused while the holder's beat is fresh
standby: took the role over, term 2
standby: stepped down, nobody holds the role
```

Run the holder, then the standby, from the folder holding both. The `sh`
lines are for a POSIX shell and the `PS>` lines for PowerShell:

{{< tabs >}}

{{< tab name="Rust" >}}
```sh
cargo run --bin holder
cargo run --bin standby
```
{{< /tab >}}

{{< tab name="Python" >}}
```sh
python holder.py
python standby.py
```
{{< /tab >}}

{{< tab name="PowerShell" >}}
```powershell
PS> pwsh -File holder.ps1
PS> pwsh -File standby.ps1
```
{{< /tab >}}

{{< tab name="C" >}}
```sh
cc holder.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o holder
cc standby.c -I/opt/subetha/include -L/opt/subetha/lib -Wl,-rpath,/opt/subetha/lib -lsubetha_ffi -o standby
./holder
./standby
```
{{< /tab >}}

{{< /tabs >}}

The Rust programs are binaries of one package that depends on
`subetha`, as [Installation](../installation/) sets up. The C programs
build against the prefix [SubEtha from C and C++](../c-and-cpp/)
installs, which also gives the CMake build and the Windows libraries.

The election lives in `role.bin`, which outlasts both processes, and
its term counts every change of holder since the file was made: run the
pair again and the terms read 3 and 4. Delete `role.bin` before a new run
to see the output above.

Each process names itself with an id. Among live holders the lowest id
wins: a claim with an id below the holder's takes the role at once, and
one above it is refused until the holder falls more than the grace period
behind, as the standby's first claim is here. A
real program passes its process id, which is what the Python and
PowerShell forms use when given none. A holder that beats more often than
the epoch is stepped keeps the role for as long as it runs, and a holder
leaving cleanly steps down, so the next claim needs no grace period at
all.

## What to do next

- [Move many items per call](../move-many-items/) sends a run of items
  through one call instead of one call an item.
- The [leader election's reference](../../reference/subetha-cxc/ownership-types/shared-leader-election/)
  gives every call in full.
