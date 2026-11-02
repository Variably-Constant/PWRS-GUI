---
title: New-GuiChannel
weight: 4
---

Makes a page's channel to its backend: two SubEtha rings, one the backend writes and the page reads, and one the page writes and the backend reads. Source: `crates/pwrs-gui/src/channel.rs`, `NewGuiChannel`; the renderer's side is `crates/pwrs-gui-web/src/page_channel.rs` and `crates/pwrs-gui-proto/src/channel.rs`, and the page's is `window.pwrs.channel` in `crates/pwrs-gui-web/src/runtime/bridge.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same.

## Syntax

```powershell
Get-Command New-GuiChannel -Syntax
```

```text
New-GuiChannel [-Name] <string> [-Text] [-Stream] [-MaxFrames <long>] [-MaxBytes <long>] [-Process <Object>] [<CommonParameters>]
```

## Description

`New-GuiChannel` makes two SubEtha adaptive rings, each of one producer and one consumer, in a folder of the channel's own under the temporary folder's `PWRS-GUI\channels`, named at random. Only this user reaches the folder: on Linux, FreeBSD and macOS its mode is 0700, and on Windows the user's temporary folder is the user's alone. The backend opens `ToPage`, which it writes, and `FromPage`, which it reads, by their paths and the channel's `Capacity`, with SubEtha's `Open-SubEthaRing -Path <path> -Capacity <capacity>`, `subetha.Ring.open`, `subetha_ring_open` or `subetha-cxc`. [Show-GuiWindow](Show-GuiWindow.md) `-Channel` gives the channel to a page, which opens it with `window.pwrs.channel(name)`. Show-GuiWindow removes the folder once the app it gave the channel to has ended; the folder of a channel no page is given is the script's to remove.

A channel carries messages unless `-Stream`. A message of any size travels as parts of one ring frame each, at most 8,192 bytes, the block size of the frame region every binding of SubEtha gives a ring. Each part's first byte says whether it is the whole message (0), a part with more to come (1) or the last part (2), and the rest of the part is the message's bytes. The page sends a message and is given one whole. With `-Text` a message's bytes are its text as UTF-8, and the page is given strings; otherwise it is given an `ArrayBuffer`, or a `Blob` when the channel's `binaryType` is `blob`. With `-Stream` each ring frame is plain bytes, with no first byte, and the page is given each frame as it comes, for a protocol that frames its own messages, as the language server protocol does.

At most `-MaxFrames` frames and `-MaxBytes` bytes are in flight each way, 64 frames and 1 MiB unless given. The page holds at most that much of the backend's traffic it has not handled, and the renderer takes no more from the ring until it has, so the backend's own sends wait once the ring is full. A send of the page's past the caps waits in the page until earlier frames are in the ring. Each ring has room for `-MaxFrames` frames, its capacity the next power of two.

With `-Process` the channel ends when that process exits: the module waits for its exit, and the renderer ends the channel in each of the app's windows, whose page's `onclose` runs. Otherwise a channel lives as long as the page keeps it open, or its window is open. A channel is open in one window at a time.

On the page `window.pwrs.channel(name)` gives a channel as a WebSocket gives itself: `readyState`, `binaryType`, `bufferedAmount`, `onopen`, `onmessage`, `onclose` and `onerror`, `send(data)` and `close()`. `send` takes a string, an `ArrayBuffer`, a typed array or a `Blob`, and returns a Promise settled once the data is in the ring. The page reaches its channels at its own origin; a page from another origin reaches them only when Show-GuiWindow has `-TrustNavigation`, and a content-security policy of your own must keep `'self'` in its `connect-src`.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiBadChannel` when empty |

The name the page opens the channel by.

### -Text

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off: the page is given bytes |
| Refused with | `GuiBadChannel` with `-Stream` |

The channel's messages are text, as UTF-8, which the page sends and is given as strings.

### -Stream

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off: the channel carries messages |

The channel carries plain frames of at most 8,192 bytes rather than messages.

### -MaxFrames, -MaxBytes

| | |
|---|---|
| Type | `System.Int64` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 64 frames, 1 MiB |
| Refused with | `GuiBadChannel` for a count that is not above 0 |

The most frames and bytes in flight each way.

### -Process

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the channel lives as long as the page keeps it |
| Refused with | `GuiBadChannel` for anything but a `System.Diagnostics.Process` |

The process whose exit ends the channel, as `Start-Process -PassThru` or `Get-Process` writes it.

## Inputs

None.

## Outputs

One `PwrsGui.Channel`:

| Property | Type | What it gives |
|---|---|---|
| `Name` | `System.String` | the name the page opens it by |
| `ToPage` | `System.String` | the path of the ring the backend writes and the page reads, which the names of SubEtha's files for the ring begin with |
| `FromPage` | `System.String` | the path of the ring the page writes and the backend reads, likewise |
| `Capacity` | `System.UInt64` | each ring's slots, which the backend opens it with |
| `Text` | `System.Boolean` | its messages are text |
| `Stream` | `System.Boolean` | it carries plain frames |
| `MaxFrames` | `System.UInt64` | the most frames in flight each way |
| `MaxBytes` | `System.UInt64` | the most bytes in flight each way |
| `Process` | `System.Object` | the process whose exit ends it; null when none |

## Examples

A channel for a language server, which frames its own messages. Its two rings are in a folder of their own:

```powershell
$ch = New-GuiChannel -Name Lsp -Stream
'{0} {1} {2} {3} {4}' -f $ch.Name, $ch.Capacity, $ch.Stream, $ch.MaxFrames, $ch.MaxBytes
$folder = Split-Path -Parent $ch.ToPage
(Test-Path -LiteralPath $folder), ((Split-Path -Parent $ch.FromPage) -eq $folder)
```

```text
Lsp 64 True 64 1048576
True
True
```

A page that opens the channel and closes with its `readyState`, which is 1 once the channel is open. The folder is gone once the app has ended:

```powershell
$page = "<script>var ch = window.pwrs.channel('Lsp'); ch.onopen = function () { window.pwrs.close(ch.readyState); };</script>"
(Show-GuiWindow -Html $page -Channel $ch).Result
Test-Path -LiteralPath $folder
```

```text
1
False
```

A backend in a script of its own, on SubEtha's module: it answers each message the page sends with the message in capitals, until the page's window closes:

<!-- skip: needs SubEtha's module and a person to close the window -->
```powershell
Import-Module SubEtha
$ch = New-GuiChannel -Name Echo -Text
$page = @'
<input id="say"><button onclick="ch.send(document.getElementById('say').value)">Send</button><p id="got"></p>
<script>
var ch = window.pwrs.channel('Echo');
ch.onmessage = function (e) { document.getElementById('got').textContent = e.data; };
</script>
'@
$backend = Start-ThreadJob {
    param($ToPage, $FromPage, $Capacity)
    Import-Module SubEtha
    $out = Open-SubEthaRing -Path $ToPage -Capacity $Capacity
    $in = Open-SubEthaRing -Path $FromPage -Capacity $Capacity
    $p = $out.RegisterProducer()
    $c = $in.RegisterConsumer()
    while ($true) {
        $frame = $in.RecvFrame($c)
        if ($null -eq $frame) { Start-Sleep -Milliseconds 20; continue }
        $text = [Text.Encoding]::UTF8.GetString($frame, 1, $frame.Length - 1).ToUpper()
        $null = $out.SendFrame($p, [byte[]](@(0) + [Text.Encoding]::UTF8.GetBytes($text)))
    }
} -ArgumentList $ch.ToPage, $ch.FromPage, $ch.Capacity
Show-GuiWindow -Html $page -Channel $ch
Stop-Job $backend
```

## Errors

`GuiBadChannel` for an empty name, `-Text` with `-Stream`, a count that is not above 0, or a `-Process` that is no process:

```powershell
try { New-GuiChannel -Name Both -Text -Stream } catch { $_.FullyQualifiedErrorId }
```

```text
GuiBadChannel,Pwrs.Modules.PWRS_GUI.NewGuiChannelCommand
```

`GuiNoChannel` when the channel's folder or rings cannot be made. The [Error Reference](Error-Reference.md) lists every error the module raises.
