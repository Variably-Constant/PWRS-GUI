---
title: Events And Waiting
weight: 4
---

How a click reaches the command that owns its window, how a command waits for it without a timer, and what each event contains. Source: `crates/pwrs-gui-proto/src/transport.rs` and `lib.rs` for the rings and messages, `crates/pwrs-gui-client/src/watch.rs` and `Window::next_event` in `lib.rs` for the waits, `arm` and `next_event` in `crates/pwrs-gui/src/session.rs` for Ctrl+C, and `crates/pwrs-gui-proto/src/clock.rs` for the clock.

## A window's events

When a command opens a window it makes a ring for that window's events and names it in the request; the renderer writes every event of that window onto that ring, and only the command reads it. An event names the widget by its number in the window's tree, says what happened, such as `Click`, `Change`, `Select` or `Activate`, includes the value, such as the rows selected, and is stamped with the moment the renderer took the input. The renderer also sends `Opened` after the window's first frame and `Closed` when it closes.

`Invoke-GuiTestEvent` raises an event in the renderer, which applies it to the widget as it would a person's input and writes it onto the window's ring like any other, so the command reads a test's click as it reads a person's.

## Waiting

A command waiting on its window sleeps until one of three things happens:

- **An event.** The ring wakes its reader when the renderer writes to it.
- **Ctrl+C.** Before it waits, the command registers a waker with PWRS, which runs when PowerShell stops the pipeline; the waker cancels the wait, and the command closes its window and returns, writing nothing.
- **The renderer's end.** When a helper exits, or the in-process renderer's run ends, the rings its commands wait on are closed, so each wakes at once and ends with `GuiRendererGone`.

Nothing polls: a command that waits an hour for a choice uses no processor time while it waits. A start waiting for the renderer's first message waits the same way, with no time limit, and Ctrl+C ends it.

## Requests and replies

A command that sends the renderer a request and waits for the reply, such as `Get-GuiWindow`, `Get-GuiSnapshot` or `Invoke-GuiTestEvent`, names a reply ring belonging to its thread in the request, and the renderer replies there. Each thread that sends requests has its own reply ring, so replies never cross between runspaces, and the thread sleeps on it as a window's command sleeps on a window's ring, Ctrl+C included.

## Large messages

A message travels in a ring's slot when it fits, and in a block of the ring's shared region when it does not. One larger than a block travels as a run of blocks the reader joins, so neither a snapshot's pixels nor a table's thousands of rows has a size limit. In-process the same messages move as values, with nothing encoded.

## The clock

Events are stamped with `QueryPerformanceCounter` on Windows and `CLOCK_MONOTONIC` elsewhere, in nanoseconds, which every process on a machine reads alike, so a time taken in PowerShell is comparable with one the helper took. `[PwrsGui.Clock]::Now()` reads it, and [Measure-GuiLatency](Measure-GuiLatency.md) uses it to time a click's journey.
