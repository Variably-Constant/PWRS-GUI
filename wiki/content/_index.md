---
title: PWRS-GUI Wiki
toc: false
---

Windows, dialogs, grids and pages of your own for Windows PowerShell 5.1 and PowerShell 7.4 or later, drawn on the GPU or in the system's webview by Rust renderers, in a helper process or on Windows in PowerShell's own. `Read-GuiChoice` shows a choice and writes the one picked, `Out-GuiGridView` shows objects in a table to pick from, and `Show-GuiWindow` shows a window of widgets or a page of your own. The other commands update, drive, capture and configure the windows a session has open, so a test can drive and read them as a person would.

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Import-Module PWRS-GUI
$target = Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production'
"Deploying to $target"
```

```text
Deploying to production
```

{{< cards >}}
  {{< card link="docs/tutorials/" title="Tutorials" subtitle="Learning-oriented. LogLens, a log viewer built in eleven chapters: a file dialog and a grid, then windows of its own, events, charts, a page, a timeline, a GPU heat map, tests and the systems it runs on." icon="academic-cap" >}}
  {{< card link="docs/how-to/" title="How-to" subtitle="Task-oriented recipes: choices, grids, pages of your own, handlers, progress, SubEtha rings and shared memory, testing a window, screenshots, where windows are drawn, latency, sharing the session, errors, and windows from Rust." icon="cog" >}}
  {{< card link="docs/explanation/" title="Explanation" subtitle="Why the module exists, where windows are drawn, how the renderers are shared, how events and waits work, web page data and recovery, the WebKitGTK, Mesa and WebView2 add-ons, and where Windows PowerShell 5.1 differs." icon="book-open" >}}
  {{< card link="docs/reference/" title="Reference" subtitle="Every command with its parameters and output types, every error id, the environment variables, the native entry's functions, and the tested platforms." icon="document-text" >}}
  {{< card link="docs/building/" title="Building From Source" subtitle="The build, the module folder it writes, and the tests, for working on the module itself." icon="code" >}}
{{< /cards >}}

## Quick links

- **New here?** [Getting Started](Getting-Started.md): install the module, pick a log with the system's file dialog and show its lines in a grid, the first of LogLens's eleven chapters.
- **Testing a script that shows a window?** [How To Test A Window](How-To-Test-A-Window.md).
- **Looking up a parameter or an output type?** The [Reference](docs/reference/), one page per command.
- **An error to look up?** [Error Reference](Error-Reference.md), by the id its `FullyQualifiedErrorId` starts with.
- **Writing a Rust module that shows windows?** [How To Share The Session With Another Module](How-To-Share-The-Session-With-Another-Module.md).

## Where the output on these pages comes from

Every example on these pages ran on Windows 11 Pro 10.0.26200 on an AMD Ryzen 9 7900X with an NVIDIA GeForce RTX 5070, at 100% display scale, in PowerShell 7.6.6 and in Windows PowerShell 5.1.26100.9444, against the module `cargo pwrs build --release` built from the code these pages describe. The output shown is PowerShell 7's; Windows PowerShell printed the same, apart from the differences each page names. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, which reach a window as a person's do. Where a page says outside an example what a run showed, it reports a run that [Tested Platforms](Tested-Platforms.md) lists with its machine and build.

## License

MIT, in [LICENSE](https://github.com/Variably-Constant/PWRS-GUI/blob/main/LICENSE) on the repository.
