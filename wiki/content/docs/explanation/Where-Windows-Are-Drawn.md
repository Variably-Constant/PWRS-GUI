---
title: Where Windows Are Drawn
weight: 2
---

The two places a window can be drawn, what each costs, and when `Auto` takes which. Source: `crates/pwrs-gui-client/src/helper.rs` and `platform.rs` for the helper, `inprocess.rs` for the in-process path, `Session::start` in `lib.rs` for the choice between them, and `crates/pwrs-gui/src/bin/pwrs-gui-egui.rs` for the helper's own side.

## The helper

The first command that opens a window, on any platform, starts `pwrs-gui-egui`, the egui renderer as a program of its own, from the copy PWRS stages for the process in the temporary folder, so a running helper locks no file in the module's folder. It draws every window of the session until the session ends.

- **Messages** cross in rings of shared memory: one the helper reads, which every thread sending to it writes, one ring per window for that window's events to the command that owns it, and one reply ring per thread that sends a request. Their names include the session's random identifier, and they are created for the current user alone: a security descriptor granting only that user on Windows, mode 0600 on Linux, FreeBSD and macOS. Nothing listens on a port or a named pipe.
- **Starting.** The helper's first message says which protocol it speaks and which GPU backend and adapter it drew with; a helper from another build is refused, naming both builds. The command that started it waits for that message with no time limit, saying so in the Verbose stream, and Ctrl+C ends the wait and the helper.
- **Ending.** The helper's standard input is a pipe the session leaves open and never writes to. When the session closes it, or PowerShell ends or is killed, the pipe ends and the helper exits. On Windows the helper is also in a job object that ends it with the PowerShell process. Its standard error goes to a log file, `PWRS-GUI/<session>.log` in the temporary folder.
- **Dying.** When a helper exits on its own, a wait on its process returns: the system thread pool's wait on Windows, and elsewhere a thread parked until the exit, which also reaps it. The session then reads the end of the helper's log and closes the rings every waiting command sleeps on. Each command wakes at once with `GuiRendererGone`, quoting how the helper ended and its log's last lines. The next window starts a new helper.

## In-process

On Windows the renderer can run in the PowerShell process instead, on a thread the module starts in a single-threaded apartment for its first in-process window. Messages move between the commands and the renderer as values over in-memory channels, with nothing encoded, and the renderer's thread wakes when one arrives. The renderer stays up between windows, as a helper does.

Removing the module closes its windows and ends the renderer's run, releasing its GPU device. The thread itself waits: a loaded copy of the module's library can build its event loop once, and a later `Import-Module` of the same build runs the renderer again on that thread. A rebuilt library loaded by `Import-Module -Force` is another copy, with a thread of its own.

In-process is Windows only.

## Which one `Auto` takes

`Auto`, the default, starts the helper. On Windows, when the helper cannot start for any reason, it draws in-process and says why in the Verbose stream; the reasons include a helper that cannot be found or launched, rings or a job object that cannot be set up, and a helper that ends before its first message. PWRS stages the helper in the temporary folder, and [AppLocker's default executable rules](https://learn.microsoft.com/windows/security/application-security/application-control/app-control-for-business/applocker/executable-rules-in-applocker) let a standard user run programs only from the Windows and Program Files folders, so where those rules are enforced the helper is refused and `Auto` draws in-process. A helper that never sends its first message is waited for, not replaced.

`Helper` and `InProcess` take one path and fail when it cannot start. [How To Choose Where Windows Are Drawn](How-To-Choose-Where-Windows-Are-Drawn.md) shows both.

## What each costs

| | Helper | In-process |
|---|---|---|
| Platforms | Windows, Linux, FreeBSD, macOS | Windows |
| A renderer that crashes | ends the helper alone; commands waiting on it wake with `GuiRendererGone`, and the next window starts a new helper | ends the PowerShell process it runs in |
| Messages | encoded and copied through shared memory | values passed in memory |
| Where policy refuses programs outside Windows and Program Files | refused | runs |
| A process to see in Task Manager | yes, `pwrs-gui-egui` | no |

[Tested Platforms](Tested-Platforms.md) has the measured times and memory of each.
