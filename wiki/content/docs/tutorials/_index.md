---
title: Tutorials
weight: 1
sidebar:
  open: true
---

LogLens, a log viewer, built a chapter at a time in PowerShell. Each chapter adds to the script the one before it ends with, and ends with the whole script so far, so read them in order.

{{< cards >}}
  {{< card link="getting-started/" title="1. Getting Started" subtitle="Install the module, pick a log with the system's file dialog, show its lines in a grid, and pick the lines you want." >}}
  {{< card link="asking-for-things/" title="2. Asking For Things" subtitle="Find out which levels to show, what to search for, and whether to open a large log, with the ready-made dialogs." >}}
  {{< card link="a-window-of-your-own/" title="3. A Window Of Your Own" subtitle="One window of widgets: a table of the log's lines, a field and a drop-down for what to show, and buttons along its bottom." >}}
  {{< card link="acting-on-events/" title="4. Acting On Events" subtitle="Filter as a person types, show the line selected, take Ctrl+F to the field, and leave the window open while the script goes on." >}}
  {{< card link="following-a-log/" title="5. Following A Log" subtitle="Follow a log as it grows, read a big log under a progress window, and put Find in a menu." >}}
  {{< card link="charts-and-trees/" title="6. Charts And Trees" subtitle="A chart of errors per minute, a tree of the log by source and level, tabs, a split and a status bar." >}}
  {{< card link="a-page-of-your-own/" title="7. A Page Of Your Own" subtitle="A summary written as HTML that tells the script the source a person clicks, and a page PowerShell sends values to." >}}
  {{< card link="a-timeline/" title="8. A Timeline" subtitle="The log's lines as dots on a canvas a person pans and zooms like a map, a click on a dot showing its line." >}}
  {{< card link="a-heat-map-on-the-gpu/" title="9. A Heat Map On The GPU" subtitle="Count the log's lines per minute and source on the GPU, and draw the counts as a heat map that follows the window's theme." >}}
  {{< card link="testing-loglens/" title="10. Testing LogLens" subtitle="Pester tests that start LogLens, act on its window as a person would, and check its lines, its status bar, the keyboard's focus and the heat map's colors." >}}
  {{< card link="renderers-modes-and-systems/" title="11. Renderers, Modes And Systems" subtitle="The same LogLens drawn by the HTML renderer, in PowerShell's own process, and in a theme and fonts of its own, and the systems it runs on." >}}
{{< /cards >}}
