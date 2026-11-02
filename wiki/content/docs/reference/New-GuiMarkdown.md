---
title: New-GuiMarkdown
weight: 31
---

Describes a Markdown document, from text or from a file. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiMarkdown`; the document's reading for both renderers, `crates/pwrs-gui-proto/src/markdown.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiMarkdown -Syntax
```

```text
New-GuiMarkdown [-Text] <string> [-OnClick <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]

New-GuiMarkdown -Path <string> [-OnClick <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiMarkdown` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a document: the text `-Text` gives, or the file `-Path` names, read as the widget is made, as UTF-8 or as UTF-16 when its byte order mark says so. In a `-Content` block `Markdown` stands for `New-GuiMarkdown`.

The document is CommonMark with tables, task lists, strikethrough, footnotes, definition lists, heading ids written `{#id}` and GitHub's alerts (`> [!NOTE]`). Fenced code is colored for Rust, C and C++, Python and TOML, and shown plain in any other language. Of raw HTML, `b`, `i`, `em`, `strong`, `br`, `sup`, `sub`, `code` and `kbd` are drawn, without their attributes; every other tag shows as text, so no script, style or frame in a document runs.

Images come from files beside the document, in the folder of `-Path`'s file or the current location for `-Text`, from absolute local paths, from `data:` addresses, and from `https` addresses the renderer fetches, in the formats [Image](New-GuiImage.md) takes. A share path is never read. An image the module cannot read is written as a warning, and one the renderer cannot fetch or draw is reported once; either shows its alt text. Until an https picture arrives, a spinner 24 points square takes its place, and a window that fits its content opens without waiting for it, sized with that square.

A link raises `Click` with its address as the event's `Value`, which `-OnClick` acts on; nothing opens on its own. A link to a heading's id scrolls the document there and raises nothing. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) clicks a link with `-Event Click -Value <address>`, and refuses an address the document has no link to. [Update-GuiWidget](Update-GuiWidget.md) sets `Text` or `Path`, not both.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Text`: position 0, mandatory |
| Pipeline input | no |
| Default | none; an empty string is allowed |

The document's text. Its relative images are found from the current location.

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | `Path`: named, mandatory |
| Pipeline input | no |
| Default | none |

The document's file, resolved from the current location as PowerShell resolves a path, with wildcards taken literally. Its relative images are found from the file's folder.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a link is clicked, with the event as its argument and as `$_`; its `Value` is the link's address.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the document, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the document.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Markdown` and its `Properties` with `Text`, or `Path` as the full path of the file.

## Examples

A document from text, its link clicked, then the window closed by its button. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Notes'; Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value 'https://example.com/notes'; Invoke-GuiTestEvent -Window $w.Id -Name Done -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Notes' -HandlerOutput Pipeline {
    Markdown "# Notes`n`nThe full notes are on [the site](https://example.com/notes)." -Name Doc -OnClick { "a link to $($_.Value)" }
    ButtonBar { Button 'Done' -Name Done -Default -Result Done }
}
$out[0]
$out[1].Result
```

```text
a link to https://example.com/notes
Done
```

A document from a file, with its full path:

```powershell
$file = Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-notes.md'
Set-Content -Path $file -Value "# Notes`n`n- [x] fetched`n- [ ] built"
$doc = New-GuiMarkdown -Path $file -Name Doc
$doc.Properties.Path -eq (Resolve-Path -LiteralPath $file).ProviderPath
$w = Show-GuiWindow -NoWait -Title 'Notes' { $doc }
Close-GuiWindow -Window $w.Id
Remove-Item -LiteralPath $file
```

```text
True
```

A file that is not there:

```powershell
try { New-GuiMarkdown -Path (Join-Path ([IO.Path]::GetTempPath()) 'no-such-notes.md') } catch { $_.FullyQualifiedErrorId }
```

```text
GuiNoFile,Pwrs.Modules.PWRS_GUI.NewGuiMarkdownCommand
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoFile` | `-Path` names no file that can be read |
| `GuiBadProperty` | `Update-GuiWidget` sets `Text` and `Path` together |

Each is described in the [Error Reference](Error-Reference.md).
