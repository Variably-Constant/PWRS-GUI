---
title: The WebKitGTK Add-On
weight: 6
---

The WebKitGTK the HTML renderer runs on Linux and FreeBSD wherever the add-on is installed, and what the add-on includes and leaves to the system. Also how a session finds it and starts the renderer from it, what happens when the GPU cannot start EGL, and what the add-on's licenses require. Source: `crates/pwrs-gui-addons` builds it, `crates/pwrs-gui-client/src/webkit.rs` finds it and starts the renderer from it, and `crates/pwrs-gui-web/src/egl.rs` checks EGL before WebKitGTK starts.

## When it is used

The HTML renderer links WebKitGTK 4.1. Where the add-on is installed, the session starts the renderer from it, so pages draw in the WebKitGTK the add-on pins whatever the system has:

1. the folder `PWRS_GUI_WEBKIT_ADDON` names, when it is set; `none` turns the add-on off and leaves the system's WebKitGTK;
2. otherwise the newest version of the module `PWRS-GUI.WebKit.linux-x64` or `PWRS-GUI.WebKit.freebsd-x64` in the folders of the session's `PSModulePath`, in order.

The add-on the first HTML renderer of a process starts from is the one every later HTML renderer of that process starts from. Without the add-on the renderer uses the system's WebKitGTK, and with neither it cannot start: an HTML window fails as [Error Reference](Error-Reference.md) says for `GuiNoWebview`, naming `Install-Module PWRS-GUI.WebKit.<rid>`. With `PWRS_GUI_TRACE` set, the session writes the add-on's folder to standard error as it starts the renderer from it.

## What it includes

| Part | Why |
|---|---|
| WebKitGTK 4.1 and every library it needs that the system does not have | the engine itself |
| WebKitGTK's web and network processes and its injected bundle, MiniBrowser left out | WebKitGTK draws and loads each page in processes of its own |
| GIO's TLS module, the one glib-networking builds, GnuTLS's on Linux and OpenSSL's on FreeBSD, and its environment-proxy module where glib-networking builds one | pages and their resources over HTTPS |
| GStreamer's core elements, its base and good plugins and its plugin scanner, but `aasink`, `cacasink`, `jack` and `video4linux2` | `audio` and `video` elements |
| the GSettings schemas, compiled | GTK reads its settings from them |
| the DejaVu fonts, and a fontconfig file that reads the system's fonts too | text on a machine with no fonts of its own |
| the MIME database, compiled, and the hicolor and Adwaita icon themes | file types and GTK's icons |
| `environment` | the variables the renderer starts with |
| `notices/` | each package's license notice |

The Linux add-on comes from Ubuntu 22.04's packages, whose C library, glibc 2.35, is the oldest PWRS's Linux builds run on; the FreeBSD add-on from FreeBSD 15.0's.

What stays the system's is what has to match the machine's kernel, display server and GPU: the C library and the C++ runtime, X11 and Wayland, and EGL, OpenGL, GBM, DRM and Vulkan with Mesa and the GPU's drivers.

The four GStreamer plugins left out are the ASCII-art video sinks, which bring the GPL-only libraries gpm and S-Lang, JACK's sink, which brings Berkeley DB under Sleepycat's license, and the camera source, which on FreeBSD brings libv4l, whose source is the Linux kernel's whole tree. WebKit picks none of the sinks, and the renderer gives no page a camera; a machine whose only sound server is JACK plays no sound in HTML windows on the add-on.

## How the renderer starts from it

WebKitGTK compiles in the folders of its processes and its injected bundle, and current releases read `WEBKIT_EXEC_PATH` only in developer builds. The add-on's copy of the library has those folders rewritten in place to paths of the same length relative to the add-on's folder: `/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1` becomes `././/lib/x86_64-linux-gnu/webkit2gtk-4.1`, and on FreeBSD `/usr/local/libexec/webkit2gtk-4.1` and `/usr/local/lib/webkit2gtk-4.1/injected-bundle` likewise. The session starts the renderer in the add-on's folder, where those paths resolve, with the variables its `environment` file names:

| Variable | Set to |
|---|---|
| `LD_LIBRARY_PATH` | the add-on's library folder, first |
| `GIO_MODULE_DIR` | its GIO modules |
| `GSETTINGS_SCHEMA_DIR`, `GSETTINGS_BACKEND` | its compiled schemas, and settings stored in memory |
| `FONTCONFIG_FILE` | its fontconfig file |
| `XDG_DATA_DIRS` | its `share` folder, first |
| `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_PLUGIN_SCANNER_1_0` | its GStreamer plugins and plugin scanner |

## When the GPU cannot start EGL

WebKitGTK's web process opens an EGL display as it starts and aborts when that fails, as some GPU drivers make it fail. Then no page draws. So before WebKitGTK starts, on Linux and FreeBSD, the HTML renderer opens a display the way the web process does, Mesa's surfaceless platform, in a child process, where a driver that crashes takes only the child down. When that fails, the renderer starts WebKitGTK on Mesa's software renderer and WebKit's shared-memory renderer (`LIBGL_ALWAYS_SOFTWARE=1`, `GALLIUM_DRIVER=llvmpipe`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`), and the session's log says why. This is true of the system's WebKitGTK as well as the add-on's. A renderer started with `LIBGL_ALWAYS_SOFTWARE=1` already skips the check.

## Licenses and source

`notices/` has each package's license notice: its Debian copyright file on Linux, its license folder from the ports tree on FreeBSD.

The module `PWRS-GUI.WebKit.<rid>.Source` has the corresponding source of every package in the add-on whose license requires it:

- on Linux, a package whose notice names the LGPL or the AGPL anywhere, or Sleepycat's license, or whose own license, that of its `Files: *` paragraph, is the GPL or the MPL with no permissive choice. A free-form notice that names the GPL in a way the build cannot settle stops the build until it is read and recorded.
- on FreeBSD, a package whose catalog entry names any of those licenses, whatever the combination: a port's list can leave out a license its code is under, as WebKitGTK's lists BSD2CLAUSE or GPLv2 though WebCore is under the LGPL.

A few notices have been read and settled by name, in place of those rules. shared-mime-info's puts its MIME database under the GPL 2 or later, so its source ships. libffi's and MIT Kerberos's name the GPL only for files their libraries do not include, so their notice is all they require. Adwaita offers every icon it installs under CC-BY-SA or the LGPL 3, and the add-on takes CC-BY-SA, which requires the attribution its notice gives and no source.

On Linux each source package comes as Ubuntu's archive has it: its `.dsc`, its upstream tarball and its Debian packaging, with `SOURCES.txt` naming the packages each one built. From gcc-12, the add-on includes only libatomic, which JavaScriptCore needs and a minimal Ubuntu lacks, so its source is libatomic's folder of the GCC tarball with GCC's build files and license texts, beside Ubuntu's packaging. On FreeBSD each package's port folder comes as the ports tree had it when the package was built, with the distribution files its `distinfo` names. Each of those comes from FreeBSD's distfile cache, or, for one the cache lacks, from the sites the port names, as the ports framework lists them, and is checked against its SHA-256.

The source is a module of its own because with it the add-on would pass 250 MB, the size NuGet.org allows a package and every PWRS-GUI package stays under; the PowerShell Gallery documents no limit of its own.
