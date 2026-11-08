---
title: The WebView2 Add-On
weight: 8
---

The WebView2 runtime the HTML renderer runs on Windows wherever the add-on is installed, what the add-on includes and what its command fetches, how a session finds the runtime, how the add-on stays current, and the license a person accepts to install it. Source: `crates/pwrs-gui-addons/src/webview2.rs` builds it, `crates/pwrs-gui-client/src/webview2.rs` finds it, and `crates/pwrs-gui-web/src/fixed.rs` starts the webviews on its runtime.

## When it is used

WebView2 comes in two forms. The evergreen runtime is installed once for the whole machine and Microsoft updates it; the HTML renderer runs on it by default. A Fixed Version runtime is a folder that an application includes and updates itself, so every page draws in the one Chromium build it pins. The add-on `PWRS-GUI.WebView2.win-x64` has the command that fetches one:

<!-- skip: installs the add-on from the PowerShell Gallery and downloads its runtime -->
```powershell
Install-Module PWRS-GUI.WebView2.win-x64
Install-GuiWebView2Runtime
```

Once its runtime is in place, the session runs the HTML renderer on it:

1. the add-on folder `PWRS_GUI_WEBVIEW2_ADDON` names, when it is set; `none` leaves the evergreen runtime;
2. otherwise the newest version of `PWRS-GUI.WebView2.win-x64` whose runtime is installed, in the first folder of the session's `PSModulePath` that has one. A version whose `Install-GuiWebView2Runtime` has not run is passed over.

`Get-GuiOption` then names the backend `WebView2 Fixed Version`, and the `Engine` of a `Get-GuiSnapshot` gives the runtime's version and its folder, the add-on's `runtime`. A runtime older than WebView2 154, the oldest the tests run, draws with a warning naming both, as the evergreen runtime does.

## The license

The add-on's package requires its license to be accepted. `Install-Module`, `Save-Module` and `Update-Module` show its `license.txt`, which says that the runtime `Install-GuiWebView2Runtime` downloads is Microsoft's, under the Microsoft Software License Terms for the Microsoft Edge WebView2 Runtime, which Microsoft shows where it publishes the runtime, at [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2), and prompt to accept them. A script passes `-AcceptLicense`; `-Force` alone is refused.

Windows PowerShell 5.1 ships with PowerShellGet 1.0.0.1, which refuses a package that requires its license to be accepted: "The specified module 'PWRS-GUI.WebView2.win-x64' with PowerShellGetFormatVersion '2.0' is not supported by the current version of PowerShellGet." [Install a package manager for PowerShell](https://learn.microsoft.com/powershell/gallery/powershellget/install-powershellget) says how to update it. With PowerShellGet 2.2.5, Windows PowerShell 5.1 prompts for the license as PowerShell 7 does.

## What Install-GuiWebView2Runtime does

The package has four files: its manifest, the script module with `Install-GuiWebView2Runtime`, `license.txt` and a `NOTICE`. The runtime itself is 705,527,045 bytes in 256 files, more than a package may contain, so the command fetches it:

1. It downloads Microsoft's cab of the runtime for x64, 310,544,938 bytes, and checks its SHA-256 against the one the add-on pins.
2. It expands the cab with Windows' `expand.exe` into a folder beside the module's `runtime` folder.
3. It grants `ALL APPLICATION PACKAGES` and `ALL RESTRICTED APPLICATION PACKAGES` read and execute on the expanded folder. WebView2's renderer runs in an app container, and Microsoft requires this grant on a Fixed Version runtime that an unpackaged application runs on Windows 10, where the renderer otherwise crashes.
4. It moves any runtime already there aside and the new one in, by renames alone, so a failed install leaves the runtime it found as it was.

On an AMD Ryzen 9 7900X under Windows 11 Pro, in PowerShell 7.6.6 and in Windows PowerShell 5.1, a first install took 7.3 to 8.7 seconds, the download included, and a reinstall with `-Force` 7.1 to 14.1.

The command does nothing while the runtime is in place, unless `-Force` is given, and `-WhatIf` says what it would do. Before fetching anything it refuses:

- a module on a network location or a UNC path, from which a Fixed Version runtime does not run;
- a `-Force` while processes run from the runtime folder, the browser processes of HTML windows drawing on it: close those windows first.

## The web data

A Fixed Version runtime stores its shared web data in `<LocalAppData>\PWRS-GUI\WebView2-<version>`, apart from the evergreen runtime's and every other version's, since a browser process refuses a profile a newer version wrote. A start removes the folders of versions no installed add-on has, once no browser process uses them. [Web Page Data And Recovery](Web-Page-Data-And-Recovery.md) says how pages share a data folder.

## Versions

The add-on's version is the runtime's: `PWRS-GUI.WebView2.win-x64` 155.0.4283.45 fetches WebView2 155.0.4283.45. A Fixed Version runtime takes no updates of its own, so a new version of the add-on follows each security update of the runtime. Microsoft's download page offers the newest patch of the two latest major versions only, so an older add-on may find its runtime gone; the command then says so, and a newer add-on fetches a newer runtime. Several versions may be installed side by side, each with a runtime of its own.

## How it is built

`pwrs-gui-addons webview2 <output folder>` writes the add-on on any system. The runtime's version, address, size and SHA-256 are pinned in the builder, read from the cab Microsoft publishes, and a new runtime is a new pin.

## The runtime's own notices

The add-on includes none of the runtime's files, and its `NOTICE` names the runtime, its address and SHA-256, and Microsoft's terms. The runtime's folder has the license files Microsoft ships in it, those of the Mu and Sigma Trust Protection Lists and of the Widevine CDM, and `show_third_party_software_licenses.bat`, which shows the credits of Chromium and the other open-source software the runtime is built from.
