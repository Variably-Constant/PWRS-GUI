<#
Unpacks each crates.io crate PWRS-GUI patches into patched/<crate>-<version>
and applies its patch from this folder, so the [patch.crates-io] entries in
the workspace's Cargo.toml find it. Every build runs this before cargo: in a
fresh checkout, cargo cannot load the workspace until it has run. It does
nothing for a crate whose folder already has the result of the same .crate
and patch, and redoes a crate whose patch has changed.

Each .crate comes from the cargo registry's cache when cargo has downloaded
it, else from static.crates.io, and must match the SHA-256 given below. The
patch is applied with git apply where git is installed, else with patch.

  pwsh -File patches/apply.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$crates = @(
    # eframe gives only its root viewport an AccessKit adapter, so no window of
    # PWRS-GUI's, each a child viewport, reaches a screen reader; the patch
    # gives each viewport's window one. It also passes a window that opens
    # inactive to a hook before it is first shown, makes no window for a
    # viewport egui has that no frame has shown, and leaves hidden a root the
    # app builds hidden, painting it as soon as a repaint is requested.
    @{ Name = 'eframe'; Version = '0.37.0'; Sha256 = '9c5642501a65ca657d05f00c643a5c14ff2346cb3818032bfe23d58e1f8e5f84'; Patch = 'eframe-0.37.0.patch' }
    # winit requests activation from macOS at launch even when it is built not
    # to activate ignoring other apps, which macOS 14 grants, so a helper
    # started with PWRS_GUI_NO_FOCUS 1 would take the front; the patch
    # requests no activation then.
    @{ Name = 'winit'; Version = '0.30.13'; Sha256 = 'a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d'; Patch = 'winit-0.30.13.patch' }
    # tao requests no activation at launch when it is set not to activate
    # ignoring other apps, as winit's patch does. On Windows, Linux and the
    # BSDs a window built not to take the focus takes none when it is first
    # shown, at its creation or later, on GTK by requesting none from the
    # window manager as it maps, so a window the web helper builds hidden
    # and shows once its page has measured it opens without the focus. On
    # Linux and the BSDs its window requests, device events and theme changes
    # reach the GTK main context through async-channel, and the BSDs set a
    # progress bar and a window on all workspaces as Linux does.
    @{ Name = 'tao'; Version = '0.37.2'; Sha256 = 'd5777b06fe6435126ae8d816eba269e050e1e10d302cf5461e9c49eccabc3548'; Patch = 'tao-0.37.2.patch' }
    # Each glyph epaint lays out names the face and glyph it is drawn from,
    # and FontsView::galley_outlines gives a galley's outlines, which a
    # canvas draws and measures its text by.
    @{ Name = 'epaint'; Version = '0.37.0'; Sha256 = '97bac86ae3dde93fc3951e790951caea739ff39a6c89d909cdaf1d9278dbb1bd'; Patch = 'epaint-0.37.0.patch' }
    # On Linux and the BSDs rfd gives a dialog the XDG portal shows a flag
    # that closes it, through the portal's Request.Close, a flag it sets
    # where the portal cannot show the dialog, in place of falling back to
    # zenity, and a flag it sets once the portal has taken the request, so
    # pwrs-gui-dialogs can close a dialog whose command stopped, lists a
    # dialog only once it shows, and runs zenity itself.
    @{ Name = 'rfd'; Version = '0.17.2'; Sha256 = '20dafead71c16a34e1ff357ddefc8afc11e7d51d6d2b9fbd07eaa48e3e540220'; Patch = 'rfd-0.17.2.patch' }
    # wgpu-core 30 refuses to configure a surface when another thread submits
    # work while the configure waits for the GPU, as the GPU layer's thread
    # does on the device egui draws with. egui-wgpu's patch catches that
    # refusal in an error scope of the UI thread's and configures again, and
    # a surface no configure has set up skips its frames, where wgpu would
    # panic at a request for one, until a later configure succeeds. On a lost
    # device, whose errors reach no scope, a configure counts only once a
    # staging write shows the device is not lost, and a frame with no
    # staging buffer for its vertices draws nothing, where egui-wgpu panics.
    @{ Name = 'egui-wgpu'; Version = '0.37.0'; Sha256 = '0309cbe1febed48af5b974e4bcc424b35e9664863a4d402697147b57a488c864'; Patch = 'egui-wgpu-0.37.0.patch' }
)

function Find-Crate([string] $File) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo' }
    $cache = Join-Path (Join-Path $cargoHome 'registry') 'cache'
    if (-not (Test-Path -LiteralPath $cache)) { return $null }
    Get-ChildItem -LiteralPath $cache -Directory | ForEach-Object { Join-Path $_.FullName $File } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}

$tar = if ($env:OS -eq 'Windows_NT') { Join-Path $env:SystemRoot 'System32\tar.exe' } else { 'tar' }
$git = Get-Command git -ErrorAction Ignore
foreach ($c in $crates) {
    $id = "$($c.Name)-$($c.Version)"
    $patch = Join-Path $PSScriptRoot $c.Patch
    $into = Join-Path $root 'patched'
    $dest = Join-Path $into $id
    $stamp = Join-Path $dest '.pwrs-gui-patch'
    $want = "$($c.Sha256) $((Get-FileHash -Algorithm SHA256 -LiteralPath $patch).Hash.ToLowerInvariant())"
    if ((Test-Path -LiteralPath $stamp) -and ([IO.File]::ReadAllText($stamp).Trim() -eq $want)) {
        "PATCHED $id, already"
        continue
    }
    $crate = Find-Crate "$id.crate"
    $downloaded = $null
    if (-not $crate) {
        $downloaded = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-$id-$PID.crate"
        if ($PSVersionTable.PSEdition -eq 'Desktop') {
            [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        }
        Invoke-WebRequest -UseBasicParsing -Uri "https://static.crates.io/crates/$($c.Name)/$id.crate" -OutFile $downloaded
        $crate = $downloaded
    }
    try {
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $crate).Hash.ToLowerInvariant()
        if ($got -ne $c.Sha256) { throw "$crate has SHA-256 $got, not $($c.Sha256)" }
        if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
        New-Item -ItemType Directory -Force -Path $into | Out-Null
        & $tar -xzf $crate -C $into
        if ($LASTEXITCODE -ne 0) { throw "unpacking $crate failed" }
    } finally {
        if ($downloaded -and (Test-Path -LiteralPath $downloaded)) { Remove-Item -LiteralPath $downloaded -Force }
    }
    if ($git) {
        & $git.Source -C $root apply "--directory=patched/$id" $patch
    } else {
        & patch -p1 -d $dest -i $patch
    }
    if ($LASTEXITCODE -ne 0) { throw "$($c.Patch) did not apply to $id" }
    [IO.File]::WriteAllText($stamp, $want)
    "PATCHED $id from $crate with $($c.Patch)"
}
