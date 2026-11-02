---
title: Get-GuiMediaPlayer
weight: 9
---

Writes where a media player of an open window is. Source: `crates/pwrs-gui/src/media.rs`, `GetGuiMediaPlayer`, `MediaPlayerState` and `GuiMediaPhase`; the renderers give the state in `Player::state` in `crates/pwrs-gui-egui/src/media.rs` and in `media` in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, after those of [New-GuiMediaPlayer](New-GuiMediaPlayer.md), [Invoke-GuiMedia](Invoke-GuiMedia.md) and [Set-GuiMediaPlayer](Set-GuiMediaPlayer.md) in the same process, which is why the window numbers run high. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from its name for unsigned integer types, `uint64` where PowerShell 7 writes `ulong`. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Get-GuiMediaPlayer -Syntax
```

```text
Get-GuiMediaPlayer [-Name] <string> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Get-GuiMediaPlayer` writes the state of the [MediaPlayer](New-GuiMediaPlayer.md) named `-Name`: where it is, its position and length, its rate, volume, mute and loop, its picture's size, whether it has sound, and why it failed. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window. The sound a player handed the system, its peak of the last half second and the seconds it has played, is in [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Sound`.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotAMediaPlayer` for a widget of another kind |

The player's name, compared without regard to case.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Aliases | `Id` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |
| Refused with | `GuiNoWindow` outside a handler when not given, or when no window described in PowerShell is open with the number it gives |

The window's number.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

A `PwrsGui.MediaPlayer`, shown in a table by its `Name`, `Phase`, `Position`, `Duration`, `Volume` and `Muted`:

| Property | Type | What it gives |
|---|---|---|
| `Name` | `System.String` | the player's name, as given |
| `Window` | `System.UInt64` | the window's number |
| `Phase` | `PwrsGui.MediaPhase` | where the player is: `Loading` while the clip opens, `Paused`, `Playing`, `Ended` once it has played to its end and stopped there, or `Failed` when it cannot be played |
| `Position` | `System.Double` | the seconds from the clip's start |
| `Duration` | `System.Nullable[System.Double]` | the clip's length in seconds; null until the player knows it |
| `Rate` | `System.Double` | how many times the clip's own speed it plays at |
| `Volume` | `System.Double` | the volume, from 0 for silence to 1 for the clip's own level |
| `Muted` | `System.Boolean` | whether the sound is silenced |
| `Loop` | `System.Boolean` | whether the clip plays again from the start each time it ends |
| `Width`, `Height` | `System.Nullable[System.UInt32]` | the picture's size in pixels; null for a clip of sound alone, and until the player knows it |
| `HasAudio` | `System.Boolean` | whether the clip has sound |
| `Error` | `System.String` | why the clip could not be played; null while it can |

## Examples

The tone the [New-GuiMediaPlayer](New-GuiMediaPlayer.md) page writes to `$wav`, once it has opened, and the keys of its sound:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Tone' { MediaPlayer $wav -AudioOnly -OnLoaded {} -Name Tone }
$null = Wait-GuiEvent -Window $w.Id -Timeout 10
Get-GuiMediaPlayer Tone -Window $w.Id | Format-List Name, Phase, Duration, HasAudio, Width, Height
(Get-GuiSnapshot -Window $w.Id).Sound.Tone.Keys -join ', '
Close-GuiWindow -Window $w.Id
```

```text
Name     : Tone
Phase    : Paused
Duration : 1
HasAudio : True
Width    :
Height   :

Peak, Seconds
```

A widget of another kind:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Not a player' { Text 'Hi' -Name Note }
try { Get-GuiMediaPlayer Note -Window $w.Id } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
Remove-Item -LiteralPath $wav
```

```text
GuiNotAMediaPlayer,Pwrs.Modules.PWRS_GUI.GetGuiMediaPlayerCommand
Note in window 16 is a Text, not a MediaPlayer
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAMediaPlayer` | the widget of that name is not a MediaPlayer; the message names its kind |
| `GuiNoWidget` | the window has no widget of that name; the message lists the names it has |
| `GuiNoWindow` | no window described in PowerShell has that number, or `-Window` is left out outside a handler |

Each ends the command. The [Error Reference](Error-Reference.md) describes each, and the session's errors, such as `GuiRendererGone`.
