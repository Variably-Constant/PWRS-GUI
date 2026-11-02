# Finds wording the documentation does not use, in the wiki, the READMEs,
# the CHANGELOG, Cargo.toml, the Rust sources, the tests, the tools, the
# examples and the workflows:
#
#   pwsh -NoProfile -File tools/Test-PlainWording.ps1
#
# Each shape below is a phrase plain English says another way: a command
# returns a value rather than "answers" it; a thing is at a place rather than
# standing, sitting, lying or falling there; a cost is not "paid" and a gain
# not "bought". Prints each hit as path:line: [shape] text, then the count of
# each shape and `HITS <n>`, and exits with the number of hits.
#
# Each line is tested joined to the next, with comment markers removed, so a
# shape wrapped across two lines of a comment or a paragraph is found; a hit
# is reported at the line it starts on. Inside a fenced block of a markdown
# file only a code comment is read. In a Rust source a byte-string literal,
# b"...", is file content, such as a format's four-byte key, and is not read.
# This file, which spells the shapes, is not read.
[CmdletBinding()]
param(
    # The repository's root.
    [string] $Root = (Join-Path $PSScriptRoot '..')
)

$ErrorActionPreference = 'Stop'
$Root = [System.IO.Path]::GetFullPath($Root)
$shapes = [ordered]@{
    'answers'        = '\banswer(s|ed|ing)\b'
    'ask of it'      = '\bas (you|it|they) asks? of it\b'
    'job of it'      = '\b(an? |the )?(earlier )?jobs? of it\b'
    'makes of'       = '\bmakes? of (it|such|them)\b'
    'reads nothing'  = '\b(reads?|reading) nothing\b'
    'stand'          = '\b(stands?|stood|standing)\b(?!\s+(in\s+for|for|out)\b)(?!-in\b)'
    'sit'            = '\b(sits?|sitting|sat)\b'
    'lie'            = '\b(lies|lie|lying|lain)\b'
    'fall'           = '\b(falls?|falling|fell|fallen)\s+(at|in|inside|into|on|onto|between|within|after|before|past|outside|against|everywhere|anywhere|where|just)\b|\bwhere(ver)?\s+(\w+\s+){1,7}(?<!\bto\s)(falls?|fell)\b(?!\s+(back|through|from|to|toward|below|under)\b)'
    'as it is there' = '\bas (it|they) (is|are) there\b'
    'pays'           = '\b(pays?|paying|paid)\b'
    'buys'           = '\b(buys?|bought|buying)\b'
    'ask X of Y'     = '\b(asks?|asked|asking)\s+(?!(?:[\w`-]+\s+){0,4}?(?:for|from|at|by|which|whether|about|was|is|are|were)\b)(?:[\w`-]+\s+){0,4}?(?<!\b(?:any|all|each|some|none|one|both|either|neither|most|many|few|several|first|last|half|rest|two|three|four|out)\s)of\s+(every|each|the|a|an|its|it|them|this|that|one|any|all)\b'
    'as it stands'   = '\bas (it|they) stands?\b'
    'parts from'     = '\bparts? from\b'
    'part company'   = '\bpart(s|ed|ing)?\s+(company|ways)\b'
    'runs on from'   = '\b(runs?|running) on from\b'
    'names say'      = '\btheir names say\b'
    'to learn that'  = '\bto learn that\b'
    'where it stops' = '\bwhere (it|they) stops?\b'
    'reaches what'   = '\breach(es)? what\b'
}
$self = [System.IO.Path]::GetFullPath($PSCommandPath)
$files = @(
    Get-ChildItem -Path (Join-Path $Root 'wiki/content') -Recurse -Filter '*.md'
    Get-Item (Join-Path $Root 'README.md')
    Get-Item (Join-Path $Root 'README.crates.md')
    Get-Item (Join-Path $Root 'CHANGELOG.md')
    Get-Item (Join-Path $Root 'Cargo.toml')
    Get-Item (Join-Path $Root 'wiki/hugo.yaml')
    Get-ChildItem -Path (Join-Path $Root 'src') -Recurse -Include '*.rs'
    Get-ChildItem -Path (Join-Path $Root 'tests') -Recurse -Include '*.ps1', '*.rs'
    Get-ChildItem -Path (Join-Path $Root 'tools') -Recurse -Include '*.ps1'
    Get-ChildItem -Path (Join-Path $Root 'examples') -Recurse -Include '*.rs'
    Get-ChildItem -Path (Join-Path $Root '.github') -Recurse -Include '*.yml', '*.yaml', '*.md'
) | Where-Object { $_.FullName -ne $self }
$options = [System.Text.RegularExpressions.RegexOptions]::IgnoreCase
$counts = [ordered]@{}
foreach ($name in $shapes.Keys) { $counts[$name] = 0 }
$total = 0
foreach ($file in $files) {
    $markdown = $file.Extension -eq '.md'
    $rust = $file.Extension -eq '.rs'
    $inFence = $false
    $bare = @(foreach ($line in [System.IO.File]::ReadAllLines($file.FullName)) {
            if ($markdown -and $line -match '^\s*```') {
                $inFence = -not $inFence
                ''
                continue
            }
            if ($inFence -and $line -notmatch '^\s*(//|# )') { ''; continue }
            if ($rust) { $line = $line -replace '\bb"(?:[^"\\]|\\.)*"', '' }
            ($line -replace '^\s*(///|//!|//|#+|\*)\s?', '').Trim()
        })
    for ($i = 0; $i -lt $bare.Count; $i++) {
        $cur = $bare[$i]
        if ($cur.Length -eq 0) { continue }
        $next = if ($i + 1 -lt $bare.Count) { $bare[$i + 1] } else { '' }
        $joined = if ($next.Length -gt 0) { $cur + ' ' + $next } else { $cur }
        # A shape inside a snake_case name is found with `_` read as a space.
        $spaced = $joined.Replace('_', ' ')
        foreach ($name in $shapes.Keys) {
            $hit = $null
            foreach ($text in @($joined, $spaced)) {
                foreach ($m in [regex]::Matches($text, $shapes[$name], $options)) {
                    if ($m.Index -lt $cur.Length) { $hit = $m; break }
                }
                if ($null -ne $hit) { break }
            }
            if ($null -eq $hit) { continue }
            $counts[$name]++
            $total++
            $shown = if ($hit.Index + $hit.Length -gt $cur.Length) { $cur + ' / ' + $next } else { $cur }
            '{0}:{1}: [{2}] {3}' -f $file.FullName.Substring($Root.Length).TrimStart('\', '/'), ($i + 1), $name, $shown
        }
    }
}
foreach ($name in $counts.Keys) { if ($counts[$name]) { '  {0,-16} {1}' -f $name, $counts[$name] } }
"HITS $total"
exit $total
