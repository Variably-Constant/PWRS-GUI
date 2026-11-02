---
title: Documentation
toc: false
sidebar:
  open: true
---

The PWRS-GUI docs are organized under the [Diataxis](https://diataxis.fr/) framework: four kinds of documentation, each for a different kind of question a reader has, and a fifth section for building the module from its repository. Every page names the source it describes, under `crates/` in the repository.

{{< cards >}}
  {{< card link="tutorials/" title="Tutorials" subtitle="Learning-oriented. LogLens, a log viewer built in eleven chapters: a file dialog and a grid, then windows of its own, events, charts, a page, a timeline, a GPU heat map, tests and the systems it runs on." icon="academic-cap" >}}
  {{< card link="how-to/" title="How-to" subtitle="Task-oriented recipes: choices, grids, pages of your own, handlers, progress, SubEtha rings and shared memory, testing a window, screenshots, where windows are drawn, latency, sharing the session, errors, and windows from Rust." icon="cog" >}}
  {{< card link="explanation/" title="Explanation" subtitle="Why the module exists, where windows are drawn, how the renderers are shared, how events and waits work, web page data and recovery, the WebKitGTK, Mesa and WebView2 add-ons, and where Windows PowerShell 5.1 differs." icon="book-open" >}}
  {{< card link="reference/" title="Reference" subtitle="Every command with its parameters and output types, every error id, the environment variables, the native entry's functions, and the tested platforms." icon="document-text" >}}
  {{< card link="building/" title="Building From Source" subtitle="What the build needs, the module folder it writes, and the tests." icon="code" >}}
{{< /cards >}}
