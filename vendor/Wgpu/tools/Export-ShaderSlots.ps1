# Writes wiki/data/shader_slots.json: where the shader code is in each
# powershell code block of the wiki, for the codeblock partial
# (wiki/layouts/_partials/components/codeblock.html) to highlight it as WGSL
# or GLSL inside the PowerShell around it.
#
#   pwsh -NoProfile -File tools/Export-ShaderSlots.ps1          write the file
#   pwsh -NoProfile -File tools/Export-ShaderSlots.ps1 -Check   exit 1 when the
#                                                               file differs from
#                                                               what a run writes
#
# Every ```powershell block under wiki/content is read as Hugo hands it to the
# partial: the lines between its fences, carriage returns removed. Its key is
# the MD5 of that text, trailing newlines cut, as hex. A block holding a
# session runs one command from each "PS> " line through the ">> " lines that
# continue it; any other block is one script. PowerShell's parser finds each
# string in a command, and a string is a slot when it holds WGSL (it has
# @compute, @vertex, @fragment, @group(, @binding(, @workgroup_size, var<, a
# struct or override declaration, or fn name(), or GLSL (a #version line), or
# when it is the expression Invoke-WgpuMap takes, first after the command's
# name or after -Expression. The lines a "PS> Get-Content <file>" command
# shows are a shader file's text: WGSL for .wgsl, GLSL for .comp, .vert,
# .frag and .glsl. A slot is one line of the string's or the file's text,
# [start, end, language], counted in characters of the block's text, as
# Hugo's substr counts them; a block with no slot has an empty list.
[CmdletBinding()]
param(
    # Exit 1, writing nothing, when the file differs from what this run writes.
    [switch] $Check
)

$ErrorActionPreference = 'Stop'
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$content = Join-Path $repo 'wiki/content'
$out = Join-Path $repo 'wiki/data/shader_slots.json'
$mapCommands = 'Invoke-WgpuMap', 'Invoke-GpuMap'
$wgslMarks = '@compute|@vertex|@fragment|@group\(|@binding\(|@workgroup_size|\bvar<|\bstruct\s+\w+\s*\{|\boverride\s+\w+|\bfn\s+\w+\s*\('
# The language of a shader file the examples show, by its extension.
$FileLanguages = @{ wgsl = 'wgsl'; comp = 'glsl'; vert = 'glsl'; frag = 'glsl'; glsl = 'glsl' }

# The ```powershell blocks of a markdown file, each as the text between its
# fences with the fence's indentation removed from each line.
function Get-PowerShellBlock([string] $Path) {
    $lines = [System.IO.File]::ReadAllText($Path).Replace("`r", '').Split("`n")
    $blocks = [System.Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -notmatch '^( {0,3})(`{3,}|~{3,})\s*([^\s`]*)') { continue }
        $indent = $Matches[1].Length
        $fence = $Matches[2]
        $lang = ($Matches[3] -split ',')[0]
        $body = [System.Collections.Generic.List[string]]::new()
        for ($i++; $i -lt $lines.Count; $i++) {
            if ($lines[$i] -match ('^ {0,3}' + [regex]::Escape([string]$fence[0]) + '{' + $fence.Length + ',}\s*$')) { break }
            $line = $lines[$i]
            $strip = 0
            while ($strip -lt $indent -and $strip -lt $line.Length -and $line[$strip] -eq ' ') { $strip++ }
            $body.Add($line.Substring($strip))
        }
        if ($lang -eq 'powershell') { $blocks.Add((($body -join "`n") + "`n")) }
    }
    , $blocks.ToArray()
}

# The commands of a block: each one's text and, for each of its lines, where
# that line starts in the block's text.
function Get-BlockCommand([string] $Block) {
    $lines = $Block.Split("`n")
    $commands = [System.Collections.Generic.List[object]]::new()
    $session = @($lines | Where-Object { $_.StartsWith('PS> ') }).Count -gt 0
    if (-not $session) {
        $starts = [System.Collections.Generic.List[int]]::new()
        $at = 0
        foreach ($line in $lines) { $starts.Add($at); $at += $line.Length + 1 }
        $commands.Add([pscustomobject]@{ Text = $Block; Starts = $starts.ToArray() })
        return , $commands.ToArray()
    }
    $at = 0
    $current = $null
    foreach ($line in $lines) {
        if ($line.StartsWith('PS> ')) {
            $current = [pscustomobject]@{ Lines = [System.Collections.Generic.List[string]]::new(); Starts = [System.Collections.Generic.List[int]]::new() }
            $current.Lines.Add($line.Substring(4))
            $current.Starts.Add($at + 4)
            $commands.Add($current)
        } elseif ($current -and $line.StartsWith('>> ')) {
            $current.Lines.Add($line.Substring(3))
            $current.Starts.Add($at + 3)
        } else {
            $current = $null
        }
        $at += $line.Length + 1
    }
    , @($commands | ForEach-Object { [pscustomobject]@{ Text = ($_.Lines -join "`n"); Starts = $_.Starts.ToArray() } })
}

# The text a string token holds, as [start, end) offsets in the command.
function Get-StringBody($Token, [string] $Text) {
    $start = $Token.Extent.StartOffset
    $end = $Token.Extent.EndOffset
    if ($Token.Kind -in 'HereStringLiteral', 'HereStringExpandable') {
        $open = $Text.IndexOf("`n", $start)
        $close = $Text.LastIndexOf("`n", $end - 1)
        if ($open -lt 0 -or $close -le $open) { return $null }
        return , @(($open + 1), $close)
    }
    , @(($start + 1), ($end - 1))
}

# The language of a string's text, or $null when it holds no shader code.
function Get-ShaderLanguage([string] $Body) {
    if ($Body -match '(?m)^\s*#version\b') { return 'glsl' }
    if ($Body -match $wgslMarks) { return 'wgsl' }
    $null
}

# The [start, end) offsets of the expression each Invoke-WgpuMap call takes.
function Get-MapExpression($Ast) {
    $found = [System.Collections.Generic.List[object]]::new()
    $calls = $Ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true)
    foreach ($call in $calls) {
        if ($call.GetCommandName() -notin $mapCommands) { continue }
        $elements = $call.CommandElements
        $expression = $null
        for ($i = 1; $i -lt $elements.Count; $i++) {
            $e = $elements[$i]
            if ($e -is [System.Management.Automation.Language.CommandParameterAst] -and $e.ParameterName -eq 'Expression') {
                $expression = if ($e.Argument) { $e.Argument } elseif ($i + 1 -lt $elements.Count) { $elements[$i + 1] } else { $null }
                break
            }
        }
        if (-not $expression -and $elements.Count -gt 1 -and $elements[1] -isnot [System.Management.Automation.Language.CommandParameterAst]) {
            $expression = $elements[1]
        }
        if ($expression -is [System.Management.Automation.Language.StringConstantExpressionAst] -or
            $expression -is [System.Management.Automation.Language.ExpandableStringExpressionAst]) {
            $found.Add(@($expression.Extent.StartOffset, $expression.Extent.EndOffset))
        }
    }
    , $found.ToArray()
}

# The character count of each UTF-16 offset of a text: a surrogate pair is
# one character, as Hugo counts them.
function Get-CharacterIndex([string] $Text) {
    $index = [int[]]::new($Text.Length + 1)
    $n = 0
    for ($i = 0; $i -lt $Text.Length; $i++) {
        $index[$i] = $n
        if (-not [char]::IsLowSurrogate($Text[$i])) { $n++ }
    }
    $index[$Text.Length] = $n
    , $index
}

# The slots of a block, as [start, end, language] in characters of its text.
function Get-BlockSlot([string] $Block) {
    $slots = [System.Collections.Generic.List[object]]::new()
    $characters = Get-CharacterIndex $Block
    foreach ($command in (Get-BlockCommand $Block)) {
        $text = $command.Text
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$errors)
        $maps = Get-MapExpression $ast
        foreach ($token in $tokens) {
            if ($token.Kind -notin 'StringLiteral', 'StringExpandable', 'HereStringLiteral', 'HereStringExpandable') { continue }
            $body = Get-StringBody $token $text
            if ($null -eq $body -or $body[1] -le $body[0]) { continue }
            $language = Get-ShaderLanguage $text.Substring($body[0], $body[1] - $body[0])
            if (-not $language -and @($maps | Where-Object { $_[0] -eq $token.Extent.StartOffset }).Count) { $language = 'wgsl' }
            if (-not $language) { continue }
            # One slot for each line of the string's text, mapped from the
            # command's offsets to the block's through each line's start.
            $lineStart = 0
            for ($k = 0; $k -lt $command.Starts.Count; $k++) {
                $lineEnd = $text.IndexOf("`n", $lineStart)
                if ($lineEnd -lt 0) { $lineEnd = $text.Length }
                $from = [math]::Max($body[0], $lineStart)
                $to = [math]::Min($body[1], $lineEnd)
                # In a double-quoted string, PowerShell's escapes and the
                # variables it expands stay PowerShell; the text between them
                # is the shader's.
                $pieces = [System.Collections.Generic.List[object]]::new()
                if ($to -gt $from -and $token.Kind -in 'StringExpandable', 'HereStringExpandable') {
                    $last = $from
                    foreach ($m in [regex]::Matches($text.Substring($from, $to - $from), '`.|\$\{[^}]*\}|\$\([^)]*\)|\$\w+')) {
                        if ($from + $m.Index -gt $last) { $pieces.Add(@($last, ($from + $m.Index))) }
                        $last = $from + $m.Index + $m.Length
                    }
                    if ($to -gt $last) { $pieces.Add(@($last, $to)) }
                } elseif ($to -gt $from) {
                    $pieces.Add(@($from, $to))
                }
                foreach ($piece in $pieces) {
                    if (-not $text.Substring($piece[0], $piece[1] - $piece[0]).Trim()) { continue }
                    $s = $command.Starts[$k] + ($piece[0] - $lineStart)
                    $e = $command.Starts[$k] + ($piece[1] - $lineStart)
                    $slots.Add(@($characters[$s], $characters[$e], $language))
                }
                $lineStart = $lineEnd + 1
            }
        }
    }
    # The lines under a "PS> Get-Content <file>" line, up to the next prompt,
    # are that file's text: a shader in the language its extension names.
    $at = 0
    $fileLanguage = $null
    foreach ($line in $Block.Split("`n")) {
        if ($line.StartsWith('PS> ')) {
            $fileLanguage = $null
            if ($line -match '^PS> Get-Content \S+\.(\w+)\s*$') { $fileLanguage = $FileLanguages[$Matches[1]] }
        } elseif ($fileLanguage -and $line.Trim()) {
            $slots.Add(@($characters[$at], $characters[$at + $line.Length], $fileLanguage))
        }
        $at += $line.Length + 1
    }
    , @($slots | Sort-Object { $_[0] })
}

$md5 = [System.Security.Cryptography.MD5]::Create()
$utf8 = [System.Text.UTF8Encoding]::new($false)
$table = [ordered]@{}
foreach ($file in Get-ChildItem -LiteralPath $content -Recurse -Filter '*.md' | Sort-Object FullName) {
    foreach ($block in (Get-PowerShellBlock $file.FullName)) {
        $key = -join ($md5.ComputeHash($utf8.GetBytes($block.TrimEnd("`n"))) | ForEach-Object { $_.ToString('x2') })
        if ($table.Contains($key)) { continue }
        $table[$key] = Get-BlockSlot $block
    }
}

$entries = foreach ($key in $table.Keys) {
    $slots = @($table[$key] | ForEach-Object { "[$($_[0]), $($_[1]), ""$($_[2])""]" }) -join ', '
    "  ""$key"": [$slots]"
}
$text = "{`n" + ($entries -join ",`n") + "`n}`n"
$slotCount = @($table.Values | ForEach-Object { $_ }).Count

if ($Check) {
    $current = if (Test-Path -LiteralPath $out) { [System.IO.File]::ReadAllText($out) } else { '' }
    if ($current -ne $text) {
        "wiki/data/shader_slots.json differs from what tools/Export-ShaderSlots.ps1 writes for the $($table.Count) powershell blocks; run it"
        exit 1
    }
    "wiki/data/shader_slots.json matches: $($table.Count) powershell blocks, $slotCount slots"
    exit 0
}
$dir = Split-Path -Parent $out
if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
[System.IO.File]::WriteAllText($out, $text, $utf8)
"wrote $($table.Count) powershell blocks, $slotCount slots"
