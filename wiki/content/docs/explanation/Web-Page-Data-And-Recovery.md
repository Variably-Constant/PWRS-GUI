---
title: Web Page Data And Recovery
weight: 5
---

Where the HTML renderer's webviews store what pages save, what processes on one data folder share, and what happens to a page whose engine ends. Source: `crates/pwrs-gui-web/src/data.rs` for the data folders, `watch_engine` and `State::engine_ended` in `crates/pwrs-gui-web/src/lib.rs` for a page whose engine ends, `save` and `restore` in `crates/pwrs-gui-web/src/runtime/bridge.js`, and `kill_on_close` in `crates/pwrs-gui-client/src/platform.rs` for the web helper's job object.

## What a webview stores

A webview stores what its pages save: cookies, local storage, IndexedDB and its cache. Every page PWRS-GUI shows, a widget window's or one of your own, comes from one origin, `pwrs://localhost`, which WebView2 names `http://pwrs.localhost`. So a page can read what every other page stored in the same place, whichever script showed it. [Set-GuiOption](Set-GuiOption.md) `-WebView` chooses the place, and the next renderer started takes the choice.

## Shared

`Shared`, the default, uses one place for every PowerShell process of the user:

| Platform | Where |
|---|---|
| Windows | `<LocalAppData>\PWRS-GUI\WebView2`; `<LocalAppData>\PWRS-GUI\WebView2-<version>` on the [WebView2 add-on](The-WebView2-Add-On.md)'s Fixed Version runtime, a folder for each version; or the folder `-WebViewDataFolder` names |
| Linux, FreeBSD | the folder WebKitGTK uses for the web helper, or the folder `-WebViewDataFolder` names |
| macOS | WebKit's default data store; `-WebViewDataFolder` is not used there, and the renderer says so when it starts |

On Windows, sharing a folder also shares processes. WebView2 runs one browser process for a data folder, with the renderer and GPU processes under it, and every webview on the folder runs under that browser process, in whichever process of the user's logon session it was made ([Process model for WebView2 apps](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model)). So:

- A process's first page joins the browser process another process started, rather than starting one of its own.
- A crash of that browser process ends every page on the folder, in every PowerShell process, and each renderer loads its pages again, as [the last section](#a-page-whose-engine-ends) says.
- The browser process belongs to no one PowerShell process. The web helper runs in a job object that ends it with its PowerShell process, and in `Shared` mode that job lets the processes the helper starts leave it, so a helper that ends does not take with it a browser process other processes' pages run under. The browser process ends by itself once its last webview has closed.
- A browser process runs with the options it was started with. A process that wants others, as with another `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, meets one of two runtimes. WebView2 154.0.4258.62 joins its page to the running browser process, and the page runs without the options it wanted. Earlier runtimes, 154.0.4258.53 among them, refuse it for as long as that browser process runs: its page fails with `GuiPageFailed`, which names `-WebView PerProcess` and `-WebViewDataFolder`. Either of those gives the process a browser process of its own, with its own options, under any runtime.
- A browser process belongs to the logon session it started in. When one user is signed in twice at once, as through a second Remote Desktop session on a server that allows it, a page in the second sign-in finds the first one's browser process on the shared folder and cannot join it. WebView2 refuses the webview at once, and the page fails with `GuiPageFailed`, which names `-WebView PerProcess` and `-WebViewDataFolder`. The first sign-in's pages go on, and either option gives the second sign-in's process a folder of its own.

On Linux and FreeBSD each web helper starts WebKitGTK processes of its own, whichever folder it stores its data in.

## PerProcess

`PerProcess` gives each renderer a place of its own, the helper's, or PowerShell's when the renderer runs in-process:

| Platform | Where |
|---|---|
| Windows | `<LocalAppData>\PWRS-GUI\WebView2-PerProcess\<pid>-<start>` |
| Linux, FreeBSD | `$XDG_DATA_HOME/PWRS-GUI/WebKitGTK-PerProcess/<pid>-<start>`, or under `~/.local/share` when `XDG_DATA_HOME` is not set |
| macOS 14 and later | a WebKit data store of the process's own, named by its id and start time |

On Windows that also means a browser process of the renderer's own: each process's first page starts one, and a crash of it ends only that process's pages. Before macOS 14, WebKit has no data store of a process's own, so the pages use the default store and the renderer says so when it starts.

A folder is named by its process's id and the time the process started. Each renderer that starts, in either mode, removes the folders and stores of processes that have ended, telling a process id the system has since given to another process by its start time. A folder that cannot be removed, as when another program has a file in it open, is named in a warning, and a later start tries again. What a page stores in `PerProcess` mode lasts as long as its renderer: the next renderer, after a reload of the module or in a new PowerShell process, starts with nothing stored.

## Which to choose

| | Shared | PerProcess |
|---|---|---|
| What pages store | read by the pages of every PowerShell process of the user | read by this renderer's pages alone, and removed after it ends |
| WebView2 browser processes | one, for every process | one per renderer |
| A process's first page, on Windows | joins the browser process running | starts a browser process, cold |
| A browser process that crashes | ends the pages of every process, each loaded again | ends this renderer's pages, each loaded again |

[Tested Platforms](Tested-Platforms.md) has the measured times of a first page in each mode, and of a page coming back after its engine ended.

## A page whose engine ends

The engine that draws a page runs in processes of its own: WebView2's browser process and the renderer processes under it, WebKitGTK's web process, WebKit's web content process on macOS. When one of them ends under a page, as when it crashes or is killed, the renderer brings the page back:

| What ended | What the renderer does |
|---|---|
| WebView2's browser process | builds a new webview in the same window, which starts a browser process or joins one, and loads the page in it |
| A WebView2 renderer process, WebKitGTK's web process, WebKit's web content process | loads the page again, which starts a new one |
| Another WebView2 process, such as its GPU process | nothing: WebView2 starts it again by itself, and the renderer notes it in the session's log |

A WebView2 renderer process or a WebKitGTK web process that stops responding is not ended: the window's command writes a warning that the page has stopped responding, once until the page responds again, and a `-NoWait` window's command writes it to standard error.

The window stays open with its place, size and title, and its command goes on waiting. What the page had comes back:

- A widget window, from `Read-GuiChoice`, `Out-GuiGridView` or `Measure-GuiLatency` drawn in HTML: the renderer stores the window's tree, every row sent to a table, and the state of each input as the page last reported it. The page loaded again opens with all of them, the same choices selected and the same text typed.
- A page of your own: what it last gave `window.pwrs.save(value)`, which `window.pwrs.restore()` returns in the page loaded again. A page that saved nothing gets `null`, as on its first load. The renderer stores the value for the life of the window, in memory: the helper's, or PowerShell's in-process.

What was under way when the engine ended is lost. A `window.pwrs.send` the page made before has no page left to reply to, and the reply `-OnMessage` gives it is dropped rather than passed to the page loaded again, whose own sends get their replies as they are made. A value `Send-GuiMessage` sends while the page loads again waits for it, as on a first load. A test event or snapshot of a widget window made as its engine ended fails, saying the window lost its page's engine; made again, it reaches the page loaded again.

Each page brought back is noted in the session's log: the helper's log file, `PWRS-GUI/<session>.log` in the temporary folder, or PowerShell's standard error in-process.

A page loaded again whose engine ends before it has drawn is not loaded a third time. The window closes, and its command fails with `GuiPageFailed`, naming what ended. A page that has drawn since its last load comes back however many times its engine ends.
