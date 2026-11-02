# The SubEtha Guide's pages as the PowerShell tests read them, by the
# rules crates/subetha-wiki-tests/src/page.rs reads them by: every fenced
# block with its language, its attributes, its tab group and tab, and the
# programs those blocks make up with the Output block that belongs to each.
# Dot-sourced from Wiki.Tests.ps1, in discovery and in its run.

$script:SEWikiContent = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot (Join-Path '..' (Join-Path '..' (Join-Path '..' (Join-Path 'wiki' 'content'))))))

# A fence's info string as its language and its attributes.
function ConvertFrom-SEWikiInfo {
    param([AllowEmptyString()][string] $Info, [string] $At)
    $brace = $Info.IndexOf('{')
    $head = if ($brace -ge 0) { $Info.Substring(0, $brace) } else { $Info }
    $lang = ($head.Trim() -split '[,\s]', 2)[0]
    $attrs = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::Ordinal)
    if ($brace -ge 0) {
        $braces = $Info.Substring($brace).TrimEnd()
        if (-not $braces.EndsWith('}')) { throw "${At}: attributes $braces are not one {...} list" }
        foreach ($m in [regex]::Matches($braces.Substring(1, $braces.Length - 2), '(\w+)=(?:"([^"]*)"|([^\s,]*))')) {
            $attrs[$m.Groups[1].Value] = if ($m.Groups[2].Success) { $m.Groups[2].Value } else { $m.Groups[3].Value }
        }
    }
    [pscustomobject]@{ Lang = $lang; Attrs = $attrs }
}

# Every fenced block of the page at $Path, in page order.
function Get-SEWikiBlocks {
    param([Parameter(Mandatory)][string] $Path)
    $lines = [System.IO.File]::ReadAllLines($Path)
    $blocks = [System.Collections.Generic.List[object]]::new()
    $group = $null
    $groups = 0
    $tab = $null
    $closed = $null
    $i = 0
    while ($i -lt $lines.Count) {
        $line = $lines[$i]
        $stripped = $line.TrimStart()
        if ($stripped.StartsWith('```')) {
            $indent = $line.Length - $stripped.Length
            $info = ConvertFrom-SEWikiInfo -Info $stripped.Substring(3) -At "${Path}:$($i + 1)"
            $body = [System.Text.StringBuilder]::new()
            $j = $i + 1
            while ($true) {
                if ($j -ge $lines.Count) { throw "${Path}:$($i + 1): a fence that never closes" }
                if ($lines[$j].Trim() -ceq '```') { break }
                $inner = $lines[$j]
                $cut = [Math]::Min($inner.Length - $inner.TrimStart().Length, $indent)
                [void]$body.Append($inner.Substring($cut)).Append("`n")
                $j++
            }
            $blocks.Add([pscustomobject]@{
                    Line       = $i + 1
                    Lang       = $info.Lang
                    Attrs      = $info.Attrs
                    Body       = $body.ToString()
                    Group      = $group
                    Tab        = $tab
                    AfterGroup = if ($null -eq $group) { $closed } else { $null }
                })
            $closed = $null
            $i = $j + 1
            continue
        }
        $shortcode = $stripped.TrimEnd()
        if ($shortcode.StartsWith('{{< tabs')) {
            $group = $groups
            $groups++
        } elseif ($shortcode -ceq '{{< /tabs >}}') {
            $closed = $group
            $group = $null
            $tab = $null
        } elseif ($shortcode.StartsWith('{{< tab ')) {
            if ($shortcode -cnotmatch 'name="([^"]*)"') { throw "${Path}:$($i + 1): a tab with no name" }
            $tab = $Matches[1]
        } elseif ($shortcode -ceq '{{< /tab >}}') {
            $tab = $null
        }
        $i++
    }
    Write-Output -NoEnumerate $blocks.ToArray()
}

# The surface a fence's language names, or $null.
function Get-SEWikiSurface {
    param([AllowEmptyString()][string] $Lang)
    switch -CaseSensitive ($Lang) {
        'rust' { 'Rust' }
        'python' { 'Python' }
        'powershell' { 'PowerShell' }
        'c' { 'C' }
        default { $null }
    }
}

# The programs of one page, each with the Output block that belongs to it.
function Get-SEWikiPrograms {
    param([Parameter(Mandatory)][string] $Page, [AllowEmptyCollection()][object[]] $Blocks)
    $programs = [System.Collections.Generic.List[object]]::new()
    $lastInTab = [System.Collections.Generic.Dictionary[string, int]]::new([System.StringComparer]::Ordinal)
    $lastOutside = $null
    foreach ($block in $Blocks) {
        if ($block.Lang -ceq 'text' -and $block.Attrs.ContainsKey('filename') -and $block.Attrs['filename'] -ceq 'Output') {
            $owners = [System.Collections.Generic.List[int]]::new()
            if ($null -ne $block.Group -and $null -ne $block.Tab) {
                $key = "$($block.Group)|$($block.Tab)"
                if ($lastInTab.ContainsKey($key)) {
                    $owners.Add($lastInTab[$key])
                    [void]$lastInTab.Remove($key)
                }
            } elseif ($null -ne $block.AfterGroup) {
                foreach ($key in @($lastInTab.Keys)) {
                    if ($key.StartsWith("$($block.AfterGroup)|", [System.StringComparison]::Ordinal)) {
                        $owners.Add($lastInTab[$key])
                        [void]$lastInTab.Remove($key)
                    }
                }
            } elseif ($null -ne $lastOutside) {
                $owners.Add($lastOutside)
                $lastOutside = $null
            }
            if ($owners.Count -eq 0) { throw "${Page}:$($block.Line): an Output block with no program before it" }
            foreach ($p in $owners) {
                $programs[$p].Output = [pscustomobject]@{ Line = $block.Line; Text = $block.Body }
            }
            continue
        }
        $surface = Get-SEWikiSurface $block.Lang
        if ($null -eq $surface -or -not $block.Attrs.ContainsKey('filename')) { continue }
        if ($null -ne $block.Tab -and $block.Tab -cne $surface) {
            throw "${Page}:$($block.Line): a $surface program in the $($block.Tab) tab"
        }
        $programs.Add([pscustomobject]@{
                Page      = $Page
                Line      = $block.Line
                Surface   = $surface
                Filename  = $block.Attrs['filename']
                Os        = if ($block.Attrs.ContainsKey('os')) { $block.Attrs['os'] } else { $null }
                Alongside = $block.Attrs.ContainsKey('alongside') -and $block.Attrs['alongside'] -ceq 'true'
                Source    = $block.Body
                Output    = $null
            })
        if ($null -ne $block.Group -and $null -ne $block.Tab) {
            $lastInTab["$($block.Group)|$($block.Tab)"] = $programs.Count - 1
        } else {
            $lastOutside = $programs.Count - 1
        }
    }
    Write-Output -NoEnumerate $programs.ToArray()
}

# Every page of the Guide as its path under the content folder, with '/'
# between the parts, and its programs; sorted by that path.
function Get-SEWikiPages {
    $root = $script:SEWikiContent
    $found = foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File -Filter '*.md') {
        $page = $file.FullName.Substring($root.Length + 1).Replace('\', '/')
        [pscustomobject]@{ Page = $page; Programs = (Get-SEWikiPrograms -Page $page -Blocks (Get-SEWikiBlocks -Path $file.FullName)) }
    }
    $found | Sort-Object -Property Page -CaseSensitive
}

# $Text as an Output block and a program's output are compared: line ends
# made `n, each line's trailing whitespace dropped, and the blank lines at
# the end dropped.
function ConvertTo-SEWikiNormalized {
    param([AllowNull()][AllowEmptyString()][string] $Text)
    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($line in ([string]$Text -split "`r?`n")) { $lines.Add($line.TrimEnd()) }
    while ($lines.Count -gt 0 -and $lines[$lines.Count - 1] -eq '') { $lines.RemoveAt($lines.Count - 1) }
    $lines -join "`n"
}

# Start $Program in $Folder as a process of $Exe, the host running the
# suite, with the temporary directory pointed at $Folder and $ModuleRoot,
# the folder holding the module's own folder, ahead on the module path.
function Start-SEWikiProgram {
    param($Program, [string] $Folder, [string] $Exe, [string] $ModuleRoot)
    $file = Join-Path $Folder $Program.Filename
    $dir = Split-Path -Parent $file
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
    [System.IO.File]::WriteAllText($file, $Program.Source, [System.Text.UTF8Encoding]::new($false))
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Exe
    $psi.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$($Program.Filename)`""
    $psi.WorkingDirectory = $Folder
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['TEMP'] = $Folder
    $psi.EnvironmentVariables['TMP'] = $Folder
    $psi.EnvironmentVariables['TMPDIR'] = $Folder
    if ([System.IO.Path]::DirectorySeparatorChar -eq '/') { $psi.EnvironmentVariables['XDG_RUNTIME_DIR'] = $Folder }
    $psi.EnvironmentVariables['PSModulePath'] = $ModuleRoot + [System.IO.Path]::PathSeparator + $env:PSModulePath
    $process = [System.Diagnostics.Process]::Start($psi)
    $process.StandardInput.Close()
    [pscustomobject]@{
        Program = $Program
        Process = $process
        Out     = $process.StandardOutput.ReadToEndAsync()
        Err     = $process.StandardError.ReadToEndAsync()
    }
}

# Remove a page's folder and everything its programs left in it, the
# per-user SubEtha folder their temporary directory held among it.
function Remove-SEWikiFolder {
    param([Parameter(Mandatory)][string] $Folder)
    Remove-Item -LiteralPath $Folder -Recurse -Force -ErrorAction Stop
}

# Wait for a started program to end, stopping it after $DeadlineSeconds,
# and answer what it did wrong against its page, or $null.
function Test-SEWikiProgram {
    param($Started, [int] $DeadlineSeconds)
    $program = $Started.Program
    $process = $Started.Process
    $at = "$($program.Page):$($program.Line) ($($program.Filename))"
    $overdue = -not $process.WaitForExit($DeadlineSeconds * 1000)
    if ($overdue) {
        $process.Kill()
        $process.WaitForExit()
    }
    $printed = ConvertTo-SEWikiNormalized $Started.Out.GetAwaiter().GetResult()
    $stderr = $Started.Err.GetAwaiter().GetResult()
    $detail = "`n--- printed`n$printed`n--- standard error`n$($stderr.TrimEnd())"
    if ($overdue) { return "${at}: still running after $DeadlineSeconds s, so stopped$detail" }
    if ($process.ExitCode -ne 0) { return "${at}: exited with $($process.ExitCode)$detail" }
    if ($null -ne $program.Output) {
        $shown = ConvertTo-SEWikiNormalized $program.Output.Text
        if ($printed -cne $shown) {
            return "${at}: printed other than the Output block at line $($program.Output.Line) shows:`n--- shown`n$shown$detail"
        }
    }
    $null
}
