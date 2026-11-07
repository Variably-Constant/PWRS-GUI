# Checks every link the built wiki writes against the files it wrote:
#
#   pwsh -NoProfile -File tools/Test-WikiLinks.ps1 -Site <built site folder> [-Readme README.md]
#
# Every href and src in every .html file under -Site that points into the
# site, relative or under -Url, is resolved against its page and must name
# a file the build wrote, in the exact case of the names on disk: GitHub
# Pages serves paths case-sensitively, while Windows matches them in any
# case. A folder link names its index.html, and a #fragment must be an id
# in the page it names. Links to other sites, another site on the same
# host among them, and mailto:, data: and javascript: links are not
# checked, nor an attribute whose value starts with &, which is markup
# shown escaped in a code example. Each Markdown link of each -Readme that
# points into the site is checked the same way.
#
# A link between pages names the other page's file, and
# wiki/layouts/_markup/render-link.html resolves it to the first page of
# that base name, so two pages under -Content with one base name are
# reported as well: a link to either would reach whichever comes first.
#
# Prints each broken link with the page it is on and each base name two
# pages share with the pages, then `LINKS <n> BROKEN <n> SHARED <n>`, and
# exits with the number broken and shared, at most 255.
[CmdletBinding()]
param(
    # The folder Hugo built the site into.
    [Parameter(Mandatory)] [string] $Site,
    # The site's base URL, as hugo.yaml gives it.
    [string] $Url = 'https://variably-constant.github.io/Wgpu/',
    # Markdown files whose links into the site are checked as well.
    [string[]] $Readme = @(),
    # The wiki's content folder, whose pages may not share a base name.
    [string] $Content = (Join-Path (Join-Path (Join-Path $PSScriptRoot '..') 'wiki') 'content')
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $Site).Path.TrimEnd('\', '/')
$base = [Uri]$Url
$basePath = if ($base.AbsolutePath.EndsWith('/')) { $base.AbsolutePath } else { $base.AbsolutePath + '/' }
$origin = $base.GetLeftPart([UriPartial]::Authority)

$files = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File) {
    [void]$files.Add($file.FullName.Substring($root.Length + 1).Replace('\', '/'))
}
$ids = @{}
$attribute = [regex]'(?i)\b(?:href|src)\s*=\s*(?:"([^"]*)"|''([^'']*)''|([^\s>"'']+))'
$idAttribute = [regex]'(?i)\bid\s*=\s*(?:"([^"]*)"|''([^'']*)''|([^\s>"'']+))'
$markdownLink = [regex]'\]\((https?://[^)\s]+)\)'

function Get-Value($match) {
    foreach ($group in $match.Groups | Select-Object -Skip 1) { if ($group.Success) { return $group.Value } }
}

function Get-PageId([string] $Relative) {
    if (-not $ids.ContainsKey($Relative)) {
        $text = [System.IO.File]::ReadAllText((Join-Path $root $Relative))
        $set = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($m in $idAttribute.Matches($text)) { [void]$set.Add((Get-Value $m)) }
        $ids[$Relative] = $set
    }
    $ids[$Relative]
}

# $null for a link outside the site, '' for a good one, else why it is broken.
function Test-Link([string] $Link, [string] $PageUrl) {
    $target = $null
    if (-not [Uri]::TryCreate([Uri]$PageUrl, $Link, [ref]$target)) { return 'not a URL' }
    if ($target.Scheme -notin 'http', 'https' -or $target.Authority -ne $base.Authority) { return $null }
    $path = [Uri]::UnescapeDataString($target.AbsolutePath)
    if (-not $path.StartsWith($basePath, [StringComparison]::Ordinal)) { return $null }
    $relative = $path.Substring($basePath.Length)
    $file = if ($relative -eq '' -or $relative.EndsWith('/')) { $relative + 'index.html' } elseif ($files.Contains($relative)) { $relative } else { $relative + '/index.html' }
    if (-not $files.Contains($file)) { return 'no such file' }
    $fragment = [Uri]::UnescapeDataString($target.Fragment.TrimStart('#'))
    if ($fragment -and $file.EndsWith('.html') -and -not (Get-PageId $file).Contains($fragment)) { return "no id $fragment" }
    ''
}

$links = 0
$broken = 0
foreach ($page in $files | Where-Object { $_.EndsWith('.html') } | Sort-Object) {
    $pageUrl = $origin + $basePath + $page
    $text = [System.IO.File]::ReadAllText((Join-Path $root $page))
    foreach ($m in $attribute.Matches($text)) {
        $link = Get-Value $m
        if ($link.StartsWith('mailto:') -or $link.StartsWith('data:') -or $link.StartsWith('javascript:') -or $link.StartsWith('&')) { continue }
        $why = Test-Link $link $pageUrl
        if ($null -eq $why) { continue }
        $links++
        if ($why) { $broken++; "BROKEN ${page}: $link ($why)" }
    }
}
foreach ($file in $Readme) {
    $text = [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $file).Path)
    foreach ($m in $markdownLink.Matches($text)) {
        $link = $m.Groups[1].Value
        $why = Test-Link $link ($origin + $basePath)
        if ($null -eq $why) { continue }
        $links++
        if ($why) { $broken++; "BROKEN ${file}: $link ($why)" }
    }
}
# A section's _index.md is not a page a link names by file.
$contentRoot = (Resolve-Path -LiteralPath $Content).Path.TrimEnd('\', '/')
$named = @{}
foreach ($md in Get-ChildItem -LiteralPath $contentRoot -Recurse -Filter '*.md' | Where-Object Name -ne '_index.md') {
    $named[$md.BaseName] += @($md.FullName.Substring($contentRoot.Length + 1).Replace('\', '/'))
}
$shared = 0
foreach ($baseName in ($named.Keys | Sort-Object)) {
    if ($named[$baseName].Count -lt 2) { continue }
    $shared++
    "SHARED ${baseName}: $(($named[$baseName] | Sort-Object) -join ', ')"
}
"LINKS $links BROKEN $broken SHARED $shared"
exit [Math]::Min($broken + $shared, 255)
