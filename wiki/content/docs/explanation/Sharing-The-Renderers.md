---
title: Sharing The Renderers
weight: 3
---

How PowerShell's runspaces, other modules and reloads share PWRS-GUI's renderers. Source: `SessionSlot` in `crates/pwrs-gui-client/src/lib.rs`, `crates/pwrs-gui/src/session.rs`, and the native entry in `crates/pwrs-gui-client/src/entry.rs` and `crates/pwrs-gui/src/entry.rs`.

## One session per renderer, every runspace

The module's library has one session per renderer for the process, egui's and HTML's, whichever runspace imported it. The first command that needs a renderer starts its session, and every later command drawing with that renderer, in that runspace or any other, uses it. Two commands that start at once start one renderer: the second waits for the first's start rather than repeating it. When that start fails, the waiting command fails with it, naming why; when it was ended by its own command's Ctrl+C, the waiting command starts a renderer itself. Reading the session takes no lock.

This is what lets a test drive a dialog: the command showing it waits in one runspace, and the test's `Get-GuiWindow`, `Invoke-GuiTestEvent` and `Get-GuiSnapshot` in another reach the same renderer. `Get-GuiWindow` lists every window both sessions have open, and a window's number says which renderer draws it, so the other two find it from the number alone.

## Windows belong to commands

A window's events go to the command that opened it, on a ring of its own, and nothing else reads them. Requests that wait for a reply, such as listing the windows or taking a snapshot, get it on a reply ring of the sending thread's own. So many runspaces can show and drive windows at once, and none waits on another's messages.

`Out-GuiGridView` without `-PassThru`, `-OutputMode` or `-Wait` returns while its window stays open; it detaches the window, so the renderer stops sending its events, and the window lives until it is closed or the session ends.

## Other modules

A PWRS module written in Rust takes one of the same sessions, naming its renderer, through PWRS-GUI's native entry, a versioned table of four functions whose addresses `Get-GuiNativeEntry` returns. It has window and reply rings of its own, numbered from a base its take was given so no two modules' windows meet, and builds the client without the renderers, so it includes none of its own. A process runs one helper, or one in-process renderer, per renderer, whichever modules open windows. [How To Share The Session With Another Module](How-To-Share-The-Session-With-Another-Module.md) shows it.

## Ending, and reloading

The sessions end once no runspace of the process has the module imported, when the last runspace that has it removes it or closes. A runspace closes when it is disposed, when a thread job or a `ForEach-Object -Parallel` run ends, and when PowerShell ends. Their windows close, each helper exits, and an in-process renderer ends its run. PowerShell does not unload a removed module's library, so importing the module again finds the same library, with no session in it, and the next window starts a renderer with the options set then.

A runspace that removes the module or closes while another has it imported leaves the sessions running, and every window open, its own included. A `-NoWait` window of a runspace that removed the module still runs its handlers there, though a PWRS-GUI command in one fails unless `PSModulePath` finds the module; a window of a runspace that closed runs them no more, and stays until it is closed.

`Import-Module -Force` runs the removal first, as PWRS does for every module, so where no other runspace has the module the sessions end the same way, and the next window starts a renderer from the library loaded then. A rebuilt library is a new copy, which PWRS loads beside the old one, with sessions of its own and native entry addresses of its own, so a module taking the session through the native entry takes one result per runspace, in that runspace.

When PowerShell itself ends, any helper ends with it.
