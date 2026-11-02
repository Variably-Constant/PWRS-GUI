---
title: How To Update The Docs
weight: 3
---

Change a page of this wiki and check it as the site will show it: regenerate the reference from the module, run the examples and write what they print into their pages, draw the pictures, build the site, and check its highlighting, its links and its wording. Every example on these pages is run, and what it prints is written into the page by a tool, never by hand. Each command below runs from the repository's root against a module folder [Building From Source](Building-From-Source.md) built.

## Regenerate the reference

After a change to a cmdlet, a class, an enumeration or an error, write the reference again:

```text
pwsh -NoProfile -File tools/Export-Reference.ps1 -Module target/pwrs/Wgpu
```

It writes the Error Reference under `wiki/content/docs/reference`, and under `reference/powershell` a page for each group of cmdlets with each one's syntax, parameters and errors, Types with every class and enumeration, and the table of every cmdlet that ends the index, below the part of it written by hand. It prints what it wrote: `wrote the Error Reference with 165 error ids, and under powershell/ 7 cmdlet group pages for 43 cmdlets, the types page with 32 classes, 9 of them with their errors, and 42 enums, and the index`. Syntax, parameter sets, positions, pipeline input and types come from the imported module by reflection. A cmdlet's text and each parameter's come from the help the module ships, which PWRS generates from the doc comments in `src/`, and a class's, a member's and an enumeration value's from the doc comment in `src/` it was generated from. The error ids, their categories and their messages are read from the calls in `src/` that raise them, and an id named anywhere else in `src/`, outside the tests, stops the run, so the Error Reference cannot miss one.

The errors each cmdlet raises, and those each class's methods throw, are written by hand in `tools/Error-Lists.psd1`: add a list there for a new cmdlet, and change one when a cmdlet raises something new. The run stops, writing nothing, when a cmdlet has no list or a list names an id nothing in `src/` raises.

The Error Reference keeps what it holds from its `## Examples` heading on: write its examples there by hand, and the next step writes what they print.

## Run the examples

```text
pwsh -NoProfile -File tools/Invoke-Documentation.ps1 -Module target/pwrs/Wgpu -Update
```

It runs every `powershell` block of the wiki and the README whose first line begins `PS> `, and writes what each command printed under it. A `PS> ` line begins a command, a `>> ` line continues it, and the lines under it, up to the next command or the end of the block, are what it printed, formatted at a width of 100 columns. A command that is only `Get-Content ./a/path` is a fixture: the lines under it are that file's text, written into the folder the examples run in before the examples below it run. A picture the page shows as `![...](/images/NAME)`, or a file it links as `[...](/images/NAME)`, is copied from `wiki/static/images` into that folder where the page names it, so an example below it loads the file the reader sees. A block after a `<!-- pwsh 7 -->` line holds what only PowerShell 7 runs.

A page whose examples are in tabs runs once for each tab name, as a reader who picks that tab reads it: everything outside the tabs, and in each `{{</* tabs */>}}` block the tab of that name, or the block's first tab where it has none of that name. An example must print the same in every reading that runs it, so a tab cannot use a variable only another tab sets.

Each page, and each reading of a page, runs in a PowerShell process of its own, in an empty `C:\Temp\wgpu-demo`, so the default device and every variable start fresh and the paths a page prints are the same on every run. Its examples run one after another in the global scope of a runspace with the console's defaults, so an error behaves as it does at the prompt, and the built module's parent folder leads `PSModulePath`, so a runspace an example starts loads the module by name. `-Page` runs the pages it names, as one comma-separated list.

Then read what changed with `git diff`. `-Update` writes whatever an example prints, an error included, and from then on the check expects it, so an error line belongs in a page only where the text above the example says the command fails.

To check the pages without writing them, leave out `-Update`: it prints each example whose output differs from what its page shows and exits with the number of pages that differ, which is how a change to the module is checked against the docs. Windows PowerShell formats tables and errors its own way, so there `-ErrorsOnly` reports only an example that ends in an error its page does not show:

```text
pwsh -NoProfile -File tools/Invoke-Documentation.ps1 -Module target/pwrs/Wgpu
powershell -NoProfile -File tools/Invoke-Documentation.ps1 -Module target/pwrs/Wgpu -ErrorsOnly
```

[How To Use The Rust Library](How-To-Use-The-Rust-Library.md) shows the output of `cargo run --example library --no-default-features`, copied from a run.

## Draw the pictures

```text
pwsh -NoProfile -File tools/Export-WikiImages.ps1 -Module target/pwrs/Wgpu
```

It draws every picture the wiki shows into `wiki/static/images`, with the shaders and the commands of the pages that show them, and the pictures the image how-tos load: a landscape and its Radiance HDR copy, a sprite, a badge, three masks of a material, and two panoramas, a sky and a studio, in Radiance HDR and OpenEXR files. The comment at the top of the script names each picture and the page that shows it. After changing the commands that make a picture, draw them again, and then run the examples again, since a page's examples load the pictures it shows.

## Build the site

The wiki is a [Hugo](https://gohugo.io/) site with the [Hextra](https://imfing.github.io/hextra/) theme, which `wiki/go.mod` takes as a Hugo module, so the build needs Hugo 0.166.0 or later, extended, and [Go](https://go.dev/) to fetch the theme.

A powershell code block is shown as a session. Each command, from its `PS> ` line through the `>> ` lines that continue it, is highlighted as PowerShell, and the lines under it are its output. The shader code in a block is highlighted as WGSL or GLSL: the lines a `Get-Content` of a `.wgsl`, `.comp`, `.vert`, `.frag` or `.glsl` file shows, a string holding shader code, and the expression `Invoke-WgpuMap` takes. `wiki/layouts/_partials/components/codeblock.html` does this from `wiki/data/shader_slots.json`, which lists where each block's shader code is. After changing a powershell block, write that file again, or the build stops at the block:

```text
pwsh -NoProfile -File tools/Export-ShaderSlots.ps1
```

`-Check` writes nothing and exits 1 when the file is out of date. Then build the site:

```text
cd wiki
hugo --gc --minify --destination <folder>
```

## Check the highlighting and the links

```text
pwsh -NoProfile -File tools/Test-WikiHighlighting.ps1 -Site <folder>
```

It prints each token a lexer could not read, which the theme paints red, and each block left unhighlighted that is not a text block, then `ERRORS <n> PLAIN <n>`, and exits with their sum.

A link from one page to another names the other page's file, as `[Getting Started](Getting-Started.md)`, and `wiki/layouts/_markup/render-link.html` resolves it to the page's address wherever the file is under `content`. Then:

```text
pwsh -NoProfile -File tools/Test-WikiLinks.ps1 -Site <folder> -Readme README.md
```

checks every link and anchor of the built site, and every link of the README into it, against the files the build wrote, in the case they have on disk, as GitHub Pages serves them. Since a link resolves to the first page with the file name it gives, it also reports two pages under `wiki/content` with the same file name. It prints each broken link with its page and each shared name with its pages, then `LINKS <n> BROKEN <n> SHARED <n>`, and exits with the number broken and shared.

## Check the wording

```text
pwsh -NoProfile -File tools/Test-PlainWording.ps1
```

It reads the wiki's pages, the README, the CHANGELOG, `Cargo.toml`, the Rust sources, the tests, the tools, the examples and the workflows for the phrases it lists, each of which has a plainer form. A phrase wrapped onto the next line is found, and in a code block of a page only the comments are read. It prints each phrase it finds with its file and line, then `HITS <n>`, and exits with that number.
