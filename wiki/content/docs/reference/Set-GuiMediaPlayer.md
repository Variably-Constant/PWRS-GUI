---
title: Set-GuiMediaPlayer
weight: 9
---

Plays, pauses, seeks and sets a media player of an open window. Source: `crates/pwrs-gui/src/media.rs`, `SetGuiMediaPlayer` and `MediaPlayerState`; the renderers make the calls in `Player::act` in `crates/pwrs-gui-egui/src/media.rs` and in `media` in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, after those of [New-GuiMediaPlayer](New-GuiMediaPlayer.md) in the same process. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from its name for unsigned integer types, `uint64` where PowerShell 7 writes `ulong`. The second example closes the first's window.

## Syntax

```powershell
Get-Command Set-GuiMediaPlayer -Syntax
```

```text
Set-GuiMediaPlayer [-Name] <string> [-Play] [-Pause] [-Position <Object>] [-Rate <double>] [-Volume <double>] [-Muted] [-Loop] [-PassThru] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Set-GuiMediaPlayer` makes calls on the [MediaPlayer](New-GuiMediaPlayer.md) named `-Name`: the loop, the mute, the volume, the rate and the position, in that order, then `-Play` or `-Pause`. Arguments out of range make no call at all. A seek goes to the frame, and a position past the clip's end goes to its end. A call stands until another call or a change [Update-GuiWidget](Update-GuiWidget.md) makes to the widget, the later winning. Inside a handler the command acts on the handler's window; elsewhere `-Window` names the window.

A call the player cannot make, such as any call on a clip that could not be played, is refused with a non-terminating error, and the calls after it are not made. With `-PassThru` the command writes the player's state once the calls are made, as [Get-GuiMediaPlayer](Get-GuiMediaPlayer.md) writes it.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoWidget` for a name the window has no widget of, `GuiNotAMediaPlayer` for a widget of another kind |

The player's name, compared without regard to case.

### -Play

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Starts playing, or plays on from where it paused; at the clip's end, plays again from its start.

### -Pause

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Pauses where it is.

### -Position

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the position is left as it is |
| Refused with | `GuiBadMedia` for a value that is neither seconds nor a `[TimeSpan]`, or below 0 |

Where to move: seconds from the clip's start, or a `[TimeSpan]`, 0 or more.

### -Rate

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the rate is left as it is |
| Refused with | `GuiBadMedia` outside 0.25 to 4 |

How many times the clip's own speed it plays at, from 0.25 to 4, the sound's pitch kept.

### -Volume

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the volume is left as it is |
| Refused with | `GuiBadMedia` outside 0 to 1 |

The volume, from 0 for silence to 1 for the clip's own level.

### -Muted

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the mute is left as it is |

Silences the sound, or with `-Muted:$false` lets it be heard again.

### -Loop

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the loop is left as it is |

Plays the clip again from its start at each end, or with `-Loop:$false` stops there.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Writes the player's state once the calls are made.

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

With `-PassThru`, a `PwrsGui.MediaPlayer`, as [Get-GuiMediaPlayer](Get-GuiMediaPlayer.md) describes it; otherwise nothing.

## Examples

The tone the [New-GuiMediaPlayer](New-GuiMediaPlayer.md) page writes to `$wav`, moved half way, sped up and quietened:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Tone' { MediaPlayer $wav -AudioOnly -OnLoaded {} -Name Tone }
$null = Wait-GuiEvent -Window $w.Id -Timeout 10
Set-GuiMediaPlayer Tone -Window $w.Id -Position 0.5 -Rate 1.5 -Volume 0.5 -PassThru | Format-List Name, Phase, Position, Rate, Volume, Muted
```

```text
Name     : Tone
Phase    : Paused
Position : 0.5
Rate     : 1.5
Volume   : 0.5
Muted    : False
```

Playing and pausing at once is refused:

```powershell
try { Set-GuiMediaPlayer Tone -Window $w.Id -Play -Pause } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
GuiBadMedia,Pwrs.Modules.PWRS_GUI.SetGuiMediaPlayerCommand
-Play starts the clip and -Pause stops it; give one
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadMedia` | `-Play` and `-Pause` together, a `-Position` that is neither seconds nor a `[TimeSpan]` or is below 0, a `-Rate` outside 0.25 to 4, or a `-Volume` outside 0 to 1; no call is made |
| `GuiRefused` | the player cannot make a call: any call on a clip that could not be played, or a rate other than 1 where GStreamer has no `scaletempo` element; the calls after it are not made |
| `GuiNotAMediaPlayer` | the widget of that name is not a MediaPlayer; the message names its kind |
| `GuiNoWidget` | the window has no widget of that name; the message lists the names it has |
| `GuiNoWindow` | no window described in PowerShell has that number, or `-Window` is left out outside a handler |

`GuiRefused` is non-terminating, the others end the command. The [Error Reference](Error-Reference.md) describes each, and the session's errors, such as `GuiRendererGone`.
