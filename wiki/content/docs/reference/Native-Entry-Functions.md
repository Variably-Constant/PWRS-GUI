---
title: The Native Entry's Functions
weight: 60
---

The contract of the four functions through which another module in the process takes one of PWRS-GUI's sessions, version 1. PWRS-GUI runs a session per renderer, egui and HTML, and a taker names the one it wants. Source: `crates/pwrs-gui-client/src/entry.rs`, where the types are declared and both sides are implemented, and `crates/pwrs-gui/src/entry.rs`, which exports them.

A module written in Rust uses `pwrs_gui_client::NativeEntry`, which follows every rule on this page, and never calls the functions itself: [How To Share The Session With Another Module](How-To-Share-The-Session-With-Another-Module.md) shows it. This page is for checking what that crate does, and for a module written in another language.

## Finding the functions

PWRS loads each module's native library from a renamed copy in a folder of the process's own, so the functions are found by address, not by name: [Get-GuiNativeEntry](Get-GuiNativeEntry.md) gives the address of each, the ABI version and the protocol. A taker checks `AbiVersion` before calling anything; `NativeEntry::from_addresses` refuses any version but 1, and any protocol but its own. An address stays callable for the life of the process.

Every symbol has its version in its name. A change to one, once a released module calls it, is a new symbol beside the old one.

## Constants

| Name | Value | Meaning |
|---|---|---|
| `ABI_VERSION` | 1 | the version these functions follow |
| `PATH_HELPER` | 1 | `InfoV1.path` of a session drawing in a helper process |
| `PATH_IN_PROCESS` | 2 | `InfoV1.path` of a session drawing in the process itself |
| `RENDERER_EGUI` | 1 | the `renderer` a `pwrs_gui_start_v1` call names for egui |
| `RENDERER_HTML` | 2 | the `renderer` a `pwrs_gui_start_v1` call names for HTML |
| `STATUS_OK` | 0 | the call did what it says |
| `STATUS_FAILED` | 1 | it failed; the error text says why |
| `STATUS_CANCELED` | 2 | the taker's cancel ended the wait |
| `STATUS_STALE` | 3 | the session named is not the one running |
| `ID_CAP` | 64 | the longest session name an `InfoV1` has room for, in bytes |

## Types

```c
typedef struct {
    uint32_t protocol;   /* the protocol the renderer speaks */
    uint32_t path;       /* PATH_HELPER or PATH_IN_PROCESS */
    uint64_t base;       /* where the taker's own window, request and ring numbers start */
    uint32_t id_len;     /* the bytes of id in use */
    uint8_t  id[64];     /* the session's name, UTF-8, which its rings' names begin with */
} InfoV1;

typedef struct {
    const void *handle;
    void (*cancel)(const void *handle);   /* ends the wait: any thread, any number of times */
    void (*release)(const void *handle);  /* gives the handle back, once; then neither is called */
} CancelV1;

typedef void *(*ArmV1)(void *ctx, CancelV1 cancel);
typedef void  (*DisarmV1)(void *ctx, void *guard);
typedef void  (*WakeV1)(const void *ctx, const uint8_t *reason, size_t reason_len);
```

Every structure is laid out as C lays it out (`#[repr(C)]`), and every function uses the C calling convention.

## The functions

### pwrs_gui_start_v1

```c
int32_t pwrs_gui_start_v1(uint32_t renderer, ArmV1 arm, DisarmV1 disarm, void *ctx, InfoV1 *info,
                          uint8_t *error, size_t error_cap, size_t *error_len);
```

Gives the session of `renderer` (`RENDERER_EGUI` or `RENDERER_HTML`) running in the process, or starts one as PWRS-GUI's options say ([Set-GuiOption](Set-GuiOption.md)) and waits for it with no time limit. Any other `renderer` fails the call with `STATUS_FAILED`. A start another caller has under way is waited on, not repeated. Before a wait, it calls `arm(ctx, cancel)`: the taker passes `cancel` to whatever ends its own waits, such as its Ctrl+C, and returns a guard, or null to refuse the wait, which fails the call; the taker releases the cancel in either case. When the wait ends, the entry calls `disarm(ctx, guard)`.

On success it writes `info` and returns `STATUS_OK`. On failure it writes as much of the reason as fits into `error`, at most `error_cap` bytes of UTF-8, writes the reason's full length to `error_len`, and returns `STATUS_FAILED`, or `STATUS_CANCELED` when the taker's cancel ended the wait.

Each successful call gives a new `base`: the index of the take, from 1, shifted left 40 bits, with bit 62 set for an HTML session. The taker numbers its windows, requests and rings from its base, and PWRS-GUI's own numbers are below the first base, so no two modules' numbers meet, and no number of one renderer's session is a number of the other's.

### pwrs_gui_watch_v1

```c
uint64_t pwrs_gui_watch_v1(const uint8_t *id, size_t id_len, WakeV1 wake, const void *ctx);
```

Registers `wake` for the end of the session `id` names, of either renderer, and returns a token for `pwrs_gui_unwatch_v1`, or 0 when no running session has that name, or `id` is not UTF-8. `wake(ctx, reason, reason_len)` is called once, from any thread, with why the renderer ended in UTF-8; for a session that has already ended, at once. `ctx` must stay usable until the unwatch returns.

### pwrs_gui_unwatch_v1

```c
void pwrs_gui_unwatch_v1(uint64_t token);
```

Removes a registered wake. A wake already running finishes before this returns, unless it is running on the calling thread.

### pwrs_gui_send_v1

```c
int32_t pwrs_gui_send_v1(const uint8_t *id, size_t id_len, const uint8_t *message, size_t len,
                         uint8_t *error, size_t error_cap, size_t *error_len);
```

Gives the in-process renderer of the session `id` names one message, encoded as `pwrs_gui_proto::transport::to_bytes` encodes a `ToHelper` (postcard). Returns `STATUS_OK`; `STATUS_STALE` when no running session has that name; or `STATUS_FAILED` with the reason written as `pwrs_gui_start_v1` writes it, naming the message it refused. A session drawing in a helper has no use for it: the taker sends on the helper's inbound ring.

## How a taker talks to the renderer

With `PATH_HELPER`, the taker opens the session's inbound ring by the name its `id` gives, as one more producer. It creates its own window ring for each window it opens and a reply ring for each of its threads that makes requests, and the helper writes that window's events and those replies onto them. With `PATH_IN_PROCESS` there is no inbound ring: the taker sends each message through `pwrs_gui_send_v1`, and the renderer still writes events and replies onto the rings the taker created. In both, the messages are those of `pwrs_gui_proto`, and a taker speaks the same `protocol` as the session or not at all.

Nothing unwinds across the boundary: each function catches a panic and reports it as `STATUS_FAILED`, a token of 0, or a line on standard error.

## A taker in Rust

The stand-in module PWRS-GUI's tests build takes the session in its `take` function, which [How To Share The Session With Another Module](How-To-Share-The-Session-With-Another-Module.md) shows whole; its last lines, from `crates/pwrs-gui-standin/src/lib.rs`:

```rust
    // SAFETY: the addresses are what PWRS-GUI's Get-GuiNativeEntry returned
    // in this process, and PWRS never unloads the library they are in.
    let native = unsafe {
        NativeEntry::from_addresses(narrow("AbiVersion")?, narrow("Protocol")?, field(entry, "StartV1")?, field(entry, "WatchV1")?, field(entry, "UnwatchV1")?, field(entry, "SendV1")?)
    }
    .map_err(gui_error)?;
    native.session(Renderer::Egui, |c: Canceler| ps.on_stop(move || c.cancel()).map_err(|e| GuiError::Arm(e.message))).map_err(gui_error)
```

`Renderer::Html` takes the HTML session instead. The taker builds `pwrs-gui-client` without its default `in-process` feature, which is what links the renderer, so it has no renderer of its own.
