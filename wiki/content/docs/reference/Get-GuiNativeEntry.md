---
title: Get-GuiNativeEntry
weight: 12
---

Gets the addresses of the native entry through which another module in the process draws with PWRS-GUI's sessions. Source: `crates/pwrs-gui/src/entry.rs`, `GetGuiNativeEntry`, `NativeEntry` and the four exported functions; the entry's contract and the taking side are `crates/pwrs-gui-client/src/entry.rs`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's. Windows PowerShell 5.1 printed the same, except where its output is shown too, apart from column widths, the addresses and the session's identifier, which differ in every process, and its names for unsigned integer types: `uint32` and `uint64` where PowerShell 7 writes `uint` and `ulong`. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows.

## Syntax

```powershell
Get-Command Get-GuiNativeEntry -Syntax
```

```text
Get-GuiNativeEntry [<CommonParameters>]
```

## Description

A module written in Rust that shows windows can draw them through PWRS-GUI's sessions instead of starting a renderer of its own, so the process runs one helper, or one in-process renderer, per renderer whichever modules open windows. It gets the entry by running `Get-GuiNativeEntry` by name from its own command, and passes the result to `pwrs_gui_client::NativeEntry::from_addresses`. `NativeEntry::session`, given `Renderer::Egui` or `Renderer::Html`, then gives it an ordinary `pwrs_gui_client::Session` over that renderer. Through it the module opens windows, waits on their events, and drives and reads them as PWRS-GUI's own commands do. Its windows appear in `Get-GuiWindow` beside PWRS-GUI's. [How To Share The Session With Another Module](How-To-Share-The-Session-With-Another-Module.md) walks through it, and [The Native Entry's Functions](Native-Entry-Functions.md) gives the contract.

PWRS loads every native library from a renamed copy in a folder of the process's own, so a module cannot find the entry by name; `Get-GuiNativeEntry` returns the addresses in the copy that is loaded. An address stays callable for the life of the process, since PWRS never frees a library it has loaded. A rebuild loaded by a later `Import-Module -Force` is a new copy with its own session, so a module used in several runspaces stores one result per runspace, taken in that runspace.

`Get-GuiNativeEntry` starts no renderer. `NativeEntry::session` returns the session of the named renderer when one is running in the process, or starts one as [Set-GuiOption](Set-GuiOption.md) says when none is, waiting for it with no time limit. The taking module passes it a callback that ends the wait on the module's own Ctrl+C.

## Parameters

`Get-GuiNativeEntry` takes only the common parameters.

## Inputs

None.

## Outputs

`PwrsGui.NativeEntry`, one:

| Property | Type | What it gives |
|---|---|---|
| `StartV1` | `System.UInt64` | the address of `pwrs_gui_start_v1` |
| `WatchV1` | `System.UInt64` | the address of `pwrs_gui_watch_v1` |
| `UnwatchV1` | `System.UInt64` | the address of `pwrs_gui_unwatch_v1` |
| `SendV1` | `System.UInt64` | the address of `pwrs_gui_send_v1` |
| `AbiVersion` | `System.UInt32` | the version of the functions' contract, 1 |
| `Protocol` | `System.UInt32` | the protocol the sessions' renderers speak; a taking module must speak the same |
| `SessionRunning` | `System.Boolean` | whether a session of either renderer is running now, read without starting one |

```powershell
Get-GuiNativeEntry | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.NativeEntry

Name           MemberType Definition
----           ---------- ----------
AbiVersion     Property   uint AbiVersion {get;set;}
Protocol       Property   uint Protocol {get;set;}
SendV1         Property   ulong SendV1 {get;set;}
SessionRunning Property   bool SessionRunning {get;set;}
StartV1        Property   ulong StartV1 {get;set;}
UnwatchV1      Property   ulong UnwatchV1 {get;set;}
WatchV1        Property   ulong WatchV1 {get;set;}
```

## Examples

The example from the command's help:

<!-- varies: addresses -->
```powershell
Get-GuiNativeEntry
```

```text
StartV1        : 140729346861408
WatchV1        : 140729346861584
UnwatchV1      : 140729346861520
SendV1         : 140729346861296
AbiVersion     : 1
Protocol       : 3
SessionRunning : False
```

The stand-in module that PWRS-GUI's tests build, `PwrsGuiStandin`, takes the session this way. It is not part of the module; its source is `crates/pwrs-gui-standin/src/lib.rs`. Its window is drawn by the renderer PWRS-GUI started, and `Get-GuiWindow` lists it:

<!-- person: $w = Wait-Window 'PWRS-GUI stand-in'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Import-Module PwrsGuiStandin
Read-StandinChoice -Choice 'red', 'green'
```

```text
green
```

<!-- varies: the session's identifier -->
```powershell
(Get-GuiNativeEntry).SessionRunning
Get-StandinSession
```

```text
True
pwrsgui40680_30b5729e061af5c5 Helper
```

## Errors

None of its own.
