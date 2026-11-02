# Runs the documented examples of the README and the wiki against a built
# module: every `powershell` block whose first line begins `PS> `.
#
#   pwsh -NoProfile -File tools/Invoke-Documentation.ps1 -Module target/pwrs/Wgpu [-Page <file>...] [-Update]
#   powershell -NoProfile -File tools/Invoke-Documentation.ps1 -Module target/pwrs/Wgpu -ErrorsOnly
#
# A `PS> ` line begins a command, a `>> ` line continues it, and the lines
# under it up to the next command or the block's end are what it writes,
# formatted at a width of 100. A command that is only `Get-Content
# ./a/path` is a fixture: the lines under it are that file's text, written
# into the folder the examples run in before any example below it. A
# picture the page shows as `![...](/images/NAME)`, or a file it links as
# `[...](/images/NAME)`, is copied from wiki/static/images into that folder
# at the place the page names it, so an example below it loads the file the
# reader sees. A block whose fence
# follows a `<!-- pwsh 7 -->` line holds what only PowerShell 7 runs, and
# Windows PowerShell passes over it. The built module's parent folder leads
# PSModulePath, so an example loads the module by name.
#
# A page whose examples are in tabs, between `{{< tab name="..." >}}` and
# `{{< /tab >}}` inside `{{< tabs >}}`, is run once for each tab name, as a
# reader who picks that tab reads it: the reading runs everything outside
# the tabs, and in each tabs block the tab of its name, or the block's first
# tab where it has none of that name. An example must write the same in
# every reading that runs it, so a tab cannot lean on what another tab ran.
#
# Each page runs in a PowerShell process of its own, so the default device
# and every variable start fresh, in an empty C:\Temp\wgpu-demo, or where
# C:\Temp is missing in a new folder that the output names as
# C:\Temp\wgpu-demo. Its examples run one after another in the global scope
# of a runspace with the console's defaults, so an error behaves as it
# does at the prompt. Without -Update the script prints, for each page, the
# examples whose output differs from what the page shows, and exits with
# the number of pages that differ. With -Update it writes each example's
# output into its page instead. With -ErrorsOnly, for Windows PowerShell
# 5.1, whose tables and errors are formatted differently, it reports only
# an example that ends in an error the page does not show.
[CmdletBinding()]
param(
    # The built module: its folder, or the Wgpu.psd1 inside it.
    [Parameter(Mandatory)] [string] $Module,
    # The pages to run; every page with an example when none is given.
    [string[]] $Page,
    # Write each example's output into its page.
    [switch] $Update,
    # Report only examples that end in an error the page does not show.
    [switch] $ErrorsOnly,
    # Run the one page given, in this process, and write the outcome as
    # JSON; the other modes start a process like this for each page and
    # each of its readings.
    [switch] $InProcess,
    # With -InProcess, the tab name of the reading to run.
    [string] $Reading = ''
)

$ErrorActionPreference = 'Stop'
$documentedRoot = 'C:\Temp\wgpu-demo'
$width = 100
# Leads the one line of a page's process that carries its outcome, so
# anything else that process writes is passed over.
$marker = '@@WGPU-DOCS@@'
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

# The examples, fixtures and pictures of the page at $Path, in page order,
# as Items, each with the range of page lines that hold what it writes, the
# number of its tabs block counted from 0 as Group and its tab's name as
# Tab, both $null outside tabs; and the tab names of each tabs block, as
# Groups.
function Read-DocumentedPage([string] $Path) {
    $lines = [System.IO.File]::ReadAllLines($Path)
    $items = [System.Collections.Generic.List[object]]::new()
    $groups = [System.Collections.Generic.List[object]]::new()
    $group = $null
    $tab = $null
    $i = 0
    while ($i -lt $lines.Count) {
        $line = $lines[$i]
        if ($line -match '^\{\{<\s*tabs\b[^>]*>\}\}\s*$') {
            if ($null -ne $group) { throw "${Path}:$($i + 1): a tabs block inside another" }
            $group = $groups.Count
            $groups.Add([System.Collections.Generic.List[string]]::new())
            $i++
            continue
        }
        if ($line -match '^\{\{<\s*/tabs\s*>\}\}\s*$') {
            if ($null -eq $group -or $null -ne $tab) { throw "${Path}:$($i + 1): a tabs block closed where none is open, or with a tab still open" }
            $group = $null
            $i++
            continue
        }
        if ($line -match '^\{\{<\s*tab\b(?:\s+name="([^"]*)")?\s*>\}\}\s*$') {
            if ($null -eq $group -or $null -ne $tab) { throw "${Path}:$($i + 1): a tab outside a tabs block, or inside another tab" }
            if (-not $Matches[1]) { throw "${Path}:$($i + 1): a tab with no name" }
            $tab = $Matches[1]
            $groups[$group].Add($tab)
            $i++
            continue
        }
        if ($line -match '^\{\{<\s*/tab\s*>\}\}\s*$') {
            if ($null -eq $tab) { throw "${Path}:$($i + 1): a tab closed where none is open" }
            $tab = $null
            $i++
            continue
        }
        if ($line.StartsWith('```') -and $line -cne '```powershell') {
            # Another language's block, read past whole.
            $i++
            while ($i -lt $lines.Count -and $lines[$i] -cne '```') { $i++ }
            $i++
            continue
        }
        if ($line -cne '```powershell') {
            foreach ($picture in [regex]::Matches($line, '!?\[[^\]]*\]\(/images/([^)\s]+)\)')) {
                $items.Add([pscustomobject]@{
                    Kind = 'Picture'; Line = $i + 1; Command = $null
                    File = $picture.Groups[1].Value
                    From = $i; To = $i; Last = $false
                    Shown = [string[]]@(); Pwsh7Only = $false
                    Group = $group; Tab = $tab
                })
            }
            $i++
            continue
        }
        $open = $i
        $end = $i + 1
        while ($end -lt $lines.Count -and $lines[$end] -cne '```') { $end++ }
        if ($end -ge $lines.Count) {
            throw "${Path}:$($open + 1): a powershell block with no closing fence"
        }
        $before = $open - 1
        while ($before -ge 0 -and $lines[$before].Trim() -eq '') { $before-- }
        $pwsh7Only = $before -ge 0 -and $lines[$before].Trim() -ceq '<!-- pwsh 7 -->'
        $j = $open + 1
        while ($j -lt $end -and $lines[$j].Trim() -eq '') { $j++ }
        if ($j -lt $end -and $lines[$j].StartsWith('PS> ')) {
            while ($j -lt $end) {
                $at = $j
                $command = $lines[$j].Substring(4)
                $j++
                while ($j -lt $end -and ($lines[$j] -ceq '>>' -or $lines[$j].StartsWith('>> '))) {
                    $command += "`n" + $(if ($lines[$j].Length -gt 3) { $lines[$j].Substring(3) } else { '' })
                    $j++
                }
                $from = $j
                $shown = [System.Collections.Generic.List[string]]::new()
                while ($j -lt $end -and -not $lines[$j].StartsWith('PS> ')) {
                    $shown.Add($lines[$j])
                    $j++
                }
                $kind = if ($command -cmatch '^Get-Content (\./[^\s''"]+)$') { 'Fixture' } else { 'Example' }
                $items.Add([pscustomobject]@{
                    Kind = $kind; Line = $at + 1; Command = $command
                    File = $(if ($kind -eq 'Fixture') { $Matches[1] })
                    From = $from; To = $j; Last = ($j -ge $end)
                    Shown = (Get-ShownLine $shown); Pwsh7Only = $pwsh7Only
                    Group = $group; Tab = $tab
                })
            }
        }
        $i = $end + 1
    }
    if ($null -ne $group) { throw "${Path}: a tabs block with no {{< /tabs >}}" }
    [pscustomobject]@{ Items = $items.ToArray(); Groups = $groups.ToArray() }
}

# The readings of a page read by Read-DocumentedPage: the name of each tab
# that holds an example, in page order, or '' alone for a page with none.
function Get-Reading($Page) {
    $names = [System.Collections.Generic.List[string]]::new()
    foreach ($it in $Page.Items) { if ($null -ne $it.Tab -and -not $names.Contains($it.Tab)) { $names.Add($it.Tab) } }
    if ($names.Count -eq 0) { return '' }
    $names
}

# Whether the reading named $Reading runs $Item of $Page: an item outside
# tabs always; one inside a tab when the tab has that name, or when it is
# the first tab of a block that has no tab of that name.
function Test-InReading($Item, $Page, [string] $Reading) {
    if ($null -eq $Item.Group) { return $true }
    $tabs = $Page.Groups[$Item.Group]
    if ($tabs -ccontains $Reading) { return $Item.Tab -ceq $Reading }
    $Item.Tab -ceq $tabs[0]
}

# Lines with each one's trailing whitespace and the blank lines before the
# first and after the last taken off, which is how a page shows output.
function Get-ShownLine([AllowNull()] [AllowEmptyCollection()] [string[]] $Lines) {
    $kept = @(foreach ($line in $Lines) { $line.TrimEnd() })
    $first = 0
    while ($first -lt $kept.Count -and $kept[$first] -eq '') { $first++ }
    $last = $kept.Count - 1
    while ($last -ge $first -and $kept[$last] -eq '') { $last-- }
    if ($last -lt $first) { return , [string[]]@() }
    , [string[]]$kept[$first..$last]
}

# What a command wrote, with the folder it ran in named as the pages name it.
function ConvertTo-ShownOutput([AllowEmptyString()] [string] $Written, [string] $Root) {
    $Written = $Written.Replace($Root, $documentedRoot).Replace($Root.Replace('\', '/'), $documentedRoot.Replace('\', '/'))
    Get-ShownLine ($Written -split "`r?`n")
}

# A record of the error, warning, verbose, debug or information stream as
# PowerShell 7's console shows it for a command typed at the prompt;
# anything else as it is. An error reads `<what failed>: <message>`, where
# what failed is chosen as the console's concise view chooses it.
function Format-StreamRecord($Record) {
    switch ($Record) {
        { $_ -is [System.Management.Automation.ErrorRecord] } {
            $myinv = $_.InvocationInfo
            $reason = 'Error'
            if ($_.Exception -and $_.Exception.WasThrownFromThrowStatement) { $reason = 'Exception' }
            elseif ($myinv.MyCommand -and $myinv.MyCommand.Name -and (Get-Command -Name $myinv.MyCommand -ErrorAction Ignore)) { $reason = $myinv.MyCommand }
            elseif ($_.CategoryInfo.Activity) { $reason = $_.CategoryInfo.Activity }
            elseif ($myinv.MyCommand) { $reason = $myinv.MyCommand }
            elseif ($myinv.InvocationName) { $reason = $myinv.InvocationName }
            elseif ($_.CategoryInfo.Category) { $reason = $_.CategoryInfo.Category }
            elseif ($_.CategoryInfo.Reason) { $reason = $_.CategoryInfo.Reason }
            $message = if ($_.ErrorDetails -and $_.ErrorDetails.Message) { $_.ErrorDetails.Message } elseif ($_.Exception) { $_.Exception.Message } else { "$_" }
            return "${reason}: $message"
        }
        { $_ -is [System.Management.Automation.WarningRecord] } { return "WARNING: $($_.Message)" }
        { $_ -is [System.Management.Automation.VerboseRecord] } { return "VERBOSE: $($_.Message)" }
        { $_ -is [System.Management.Automation.DebugRecord] } { return "DEBUG: $($_.Message)" }
        { $_ -is [System.Management.Automation.InformationRecord] } { return "$($_.MessageData)" }
    }
    $Record
}

# Runs $Command in $Runspace's global scope, and returns what it wrote to
# every stream, in order. A terminating error ends it, as it ends a command
# typed at the prompt, and comes last.
function Invoke-InRunspace($Runspace, [string] $Command) {
    $ps = [powershell]::Create()
    $ps.Runspace = $Runspace
    [void]$ps.AddScript(". {`n$Command`n} *>&1", $false)
    $output = [System.Management.Automation.PSDataCollection[psobject]]::new()
    try {
        [void]$ps.Invoke($null, $output)
    } catch {
        $inner = $_.Exception.InnerException
        if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { $output.Add($inner.ErrorRecord) } else { $output.Add($_) }
    } finally {
        $ps.Dispose()
    }
    $output
}

# Runs the reading $Reading of the page at $Path in a new folder, against
# the module $Manifest imported into a runspace of the page's own, and
# returns the output of each example the reading runs.
function Invoke-DocumentedPage([string] $Path, [string] $Manifest, [string] $Reading) {
    $docPage = Read-DocumentedPage $Path
    $docItems = @($docPage.Items | Where-Object { Test-InReading -Item $_ -Page $docPage -Reading $Reading })
    # The examples run in the folder the pages name where it can be made,
    # so a table that shows a path is laid out for the path shown. A file
    # in it marks it as this script's, which removes no folder without it.
    $docRoot = if (Test-Path -LiteralPath (Split-Path -Parent $documentedRoot)) { $documentedRoot } else {
        Join-Path ([System.IO.Path]::GetTempPath()) ('wgpu-docs-' + [guid]::NewGuid().ToString('N'))
    }
    $docMark = Join-Path $docRoot '.wgpu-docs'
    if (Test-Path -LiteralPath $docRoot) {
        if (-not (Test-Path -LiteralPath $docMark)) { throw "$docRoot exists and this script did not make it; move it away and run again" }
        Remove-Item -LiteralPath $docRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $docRoot | Out-Null
    New-Item -ItemType File -Path $docMark | Out-Null
    if ($null -ne (Get-Variable -Name PSStyle -ErrorAction Ignore)) { $PSStyle.OutputRendering = 'PlainText' }
    [System.Environment]::CurrentDirectory = $docRoot
    $docUtf8 = [System.Text.UTF8Encoding]::new($false)
    $runspace = [runspacefactory]::CreateRunspace($Host, [initialsessionstate]::CreateDefault2())
    # Every example on the page runs on one thread, as at the console, so a
    # thread that used a backend's driver lives until the process ends.
    $runspace.ThreadOptions = [System.Management.Automation.Runspaces.PSThreadOptions]::ReuseThread
    $runspace.Open()
    try {
        $quoted = { param($s) "'" + $s.Replace("'", "''") + "'" }
        $setup = @(Invoke-InRunspace $runspace (
                "Set-Location -LiteralPath $(& $quoted $docRoot)`n" +
                "Import-Module $(& $quoted $Manifest)`n" +
                "if (`$null -ne (Get-Variable -Name PSStyle -ErrorAction Ignore)) { `$PSStyle.OutputRendering = 'PlainText' }"))
        if ($setup.Count) { throw "the examples' runspace cannot be prepared: $(($setup | ForEach-Object { Format-StreamRecord $_ }) -join '; ')" }
        foreach ($docItem in $docItems) {
            if ($docItem.Kind -eq 'Fixture') {
                $docFile = Join-Path $docRoot $docItem.File.Substring(2)
                $docDir = Split-Path -Parent $docFile
                if (-not (Test-Path -LiteralPath $docDir)) { New-Item -ItemType Directory -Path $docDir | Out-Null }
                [System.IO.File]::WriteAllText($docFile, ($docItem.Shown -join "`n") + "`n", $docUtf8)
                $docItem | Add-Member -NotePropertyName Written -NotePropertyValue $docItem.Shown
                continue
            }
            if ($docItem.Kind -eq 'Picture') {
                $docSource = Join-Path $repo ('wiki/static/images/' + $docItem.File)
                if (-not (Test-Path -LiteralPath $docSource -PathType Leaf)) {
                    throw "${Path}:$($docItem.Line): the page shows /images/$($docItem.File), which wiki/static/images does not hold"
                }
                $docFile = Join-Path $docRoot $docItem.File
                $docDir = Split-Path -Parent $docFile
                if (-not (Test-Path -LiteralPath $docDir)) { New-Item -ItemType Directory -Path $docDir | Out-Null }
                Copy-Item -LiteralPath $docSource -Destination $docFile -Force
                $docItem | Add-Member -NotePropertyName Written -NotePropertyValue $docItem.Shown
                continue
            }
            if ($docItem.Pwsh7Only -and $PSVersionTable.PSEdition -eq 'Desktop') {
                $docItem | Add-Member -NotePropertyName Written -NotePropertyValue $docItem.Shown
                continue
            }
            $docText = Invoke-InRunspace $runspace $docItem.Command | ForEach-Object { Format-StreamRecord $_ } | Out-String -Width $width
            $docItem | Add-Member -NotePropertyName Written -NotePropertyValue (ConvertTo-ShownOutput $docText $docRoot)
        }
    } finally {
        $runspace.Dispose()
        [System.Environment]::CurrentDirectory = [System.IO.Path]::GetTempPath()
        Remove-Item -LiteralPath $docRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    $docItems
}

$utf8 = [System.Text.UTF8Encoding]::new($false)
if ($InProcess) {
    # The outcome reaches the parent as UTF-8, which it reads as UTF-8.
    [Console]::OutputEncoding = $utf8
    $item = Get-Item -LiteralPath $Module
    $manifest = if ($item.PSIsContainer) { Join-Path $item.FullName 'Wgpu.psd1' } else { $item.FullName }
    $env:PSModulePath = (Split-Path -Parent (Split-Path -Parent $manifest)) + [System.IO.Path]::PathSeparator + $env:PSModulePath
    # Imported here as well, for the formatting of what the examples write.
    Import-Module $manifest
    $results = @(Invoke-DocumentedPage -Path $Page[0] -Manifest $manifest -Reading $Reading | ForEach-Object {
        [pscustomobject]@{ Kind = $_.Kind; Line = $_.Line; Command = $_.Command; From = $_.From; To = $_.To; Last = $_.Last; Shown = @($_.Shown); Written = @($_.Written) }
    })
    $marker + (ConvertTo-Json -InputObject $results -Depth 5 -Compress)
    return
}

$pages = if ($Page) {
    # `pwsh -File` passes a comma-separated list as one string.
    @($Page | ForEach-Object { $_ -split ',' } | Where-Object { $_ } | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
} else {
    $content = Join-Path $repo 'wiki/content'
    @(@(Join-Path $repo 'README.md') + @(Get-ChildItem -LiteralPath $content -Recurse -Filter '*.md' | ForEach-Object FullName) |
        Where-Object { Select-String -LiteralPath $_ -Pattern '^PS> ' -CaseSensitive -Quiet })
}
$moduleFull = (Resolve-Path -LiteralPath $Module).Path
$hostExe = (Get-Process -Id $PID).Path
$differing = 0
# What an error looks like in a page: `<what failed>: <message>`, where
# what failed is a command, an exception's type or an error category.
$errorLine = '(?m)^([A-Za-z]+-[A-Za-z]+|\w*Exception|' + ([enum]::GetNames([System.Management.Automation.ErrorCategory]) -join '|') + '): '
# Each page's process writes UTF-8, and PowerShell decodes what a native
# command writes with the console's output encoding, which is the whole
# console's and is put back at the end.
$consoleEncoding = [Console]::OutputEncoding
[Console]::OutputEncoding = $utf8
try {
    foreach ($path in $pages) {
        $name = if ($path.StartsWith($repo + [System.IO.Path]::DirectorySeparatorChar)) { $path.Substring($repo.Length + 1) } else { $path }
        $readings = @(Get-Reading (Read-DocumentedPage $path))
        # Each example's outcome in the first reading that ran it, by line.
        $examples = [ordered]@{}
        $failures = [System.Collections.Generic.List[string]]::new()
        foreach ($reading in $readings) {
            $label = if ($readings.Count -gt 1) { " ($reading)" } else { '' }
            $readingArgs = if ($reading) { @('-Reading', $reading) } else { @() }
            $written = @(& $hostExe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $PSCommandPath -Module $moduleFull -Page $path @readingArgs -InProcess)
            $json = @($written | Where-Object { $_ -is [string] -and $_.StartsWith($marker) })
            if ($LASTEXITCODE -ne 0 -or $json.Count -ne 1) {
                throw "running $name$label failed with exit code ${LASTEXITCODE}:`n$($written -join "`n")"
            }
            # Windows PowerShell 5.1's ConvertFrom-Json writes an array as one
            # object; the ForEach-Object unrolls it there and changes nothing in 7.
            $results = @(ConvertFrom-Json $json[0].Substring($marker.Length) | ForEach-Object { $_ })
            foreach ($r in ($results | Where-Object Kind -eq 'Example')) {
                $shown = @($r.Shown) -join "`n"
                $got = @($r.Written) -join "`n"
                $key = [string]$r.Line
                if ($examples.Contains($key)) {
                    $first = $examples[$key]
                    $before = @($first.Written) -join "`n"
                    if ($before -cne $got) {
                        $failures.Add("line $($r.Line): PS> $($r.Command)`n--- written in the $($first.Reading) reading`n$before`n--- written in the $reading reading`n$got")
                    }
                    continue
                }
                $r | Add-Member -NotePropertyName Reading -NotePropertyValue $reading
                $examples[$key] = $r
                if ($Update) { continue }
                if ($ErrorsOnly) {
                    $erred = $got -match $errorLine -and $shown -notmatch $errorLine
                    if (-not $erred) { continue }
                } elseif ($shown -ceq $got) {
                    continue
                }
                $failures.Add("line $($r.Line)${label}: PS> $($r.Command)`n--- shown`n$shown`n--- written`n$got")
            }
        }
        $count = "$($examples.Count) examples" + $(if ($readings.Count -gt 1) { ", read as $($readings -join ' and ')" } else { '' })
        if ($Update) {
            $lines = [System.Collections.Generic.List[string]]([System.IO.File]::ReadAllLines($path))
            $changed = 0
            foreach ($r in ($examples.Values | Sort-Object From -Descending)) {
                $new = [System.Collections.Generic.List[string]]::new()
                foreach ($line in @($r.Written)) { $new.Add($line) }
                if ($new.Count -gt 0 -and -not $r.Last) { $new.Add('') }
                if (((@($r.Shown) -join "`n") -cne (@($r.Written) -join "`n"))) { $changed++ }
                $lines.RemoveRange($r.From, $r.To - $r.From)
                $lines.InsertRange($r.From, $new)
            }
            # LF, as tools/Export-Reference.ps1 writes the pages it generates.
            [System.IO.File]::WriteAllText($path, ($lines -join "`n") + "`n", $utf8)
            "$name`: $count, $changed updated"
            if ($failures.Count) {
                $differing++
                $failures -join "`n`n"
            }
            continue
        }
        if ($failures.Count) {
            $differing++
            "== $name"
            $failures -join "`n`n"
        } else {
            "ok $name ($count)"
        }
    }
} finally {
    [Console]::OutputEncoding = $consoleEncoding
}
exit $differing
