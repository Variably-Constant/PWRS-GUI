# Checks the code highlighting of the built wiki.
#
#   pwsh -NoProfile -File tools/Test-WikiHighlighting.ps1 -Site <built site>
#
# Reads every .html file under the built site and, for each code block, its
# language and whether Chroma highlighted it. Chroma marks text a lexer could
# not read with the err class, which the theme paints red: each such token is
# printed with its page, the block's language and its line. A block Chroma did
# not highlight, other than a text block, is printed with its page and first
# line. Prints the number of blocks of each language, then
# `ERRORS <n> PLAIN <n>`, and exits with their sum.
[CmdletBinding()]
param(
    # The folder Hugo built the site into.
    [Parameter(Mandatory)] [string] $Site
)

$ErrorActionPreference = 'Stop'
$root = (Get-Item -LiteralPath $Site).FullName
# A minified site writes attribute values without quotes, so each pattern
# takes a value quoted or bare.
$block = [regex]'(?s)<pre\b([^>]*)>\s*<code\b([^>]*)>(.*?)</code>\s*</pre>'
$langOf = [regex]'data-lang=["'']?([^"''\s>]+)'
$langClass = [regex]'class=["'']?language-([^"''\s>]+)'
$lineStart = [regex]'<span class=["'']?line["'']?>'
$errToken = [regex]'(?s)<span class=["'']?err["'']?>(.*?)</span>'

function Get-PlainText([string] $Fragment) { [System.Net.WebUtility]::HtmlDecode(($Fragment -replace '<[^>]+>', '')) }

$census = @{}
$errors = [System.Collections.Generic.List[string]]::new()
$plain = [System.Collections.Generic.List[string]]::new()
$pages = 0
foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -Filter '*.html' | Sort-Object FullName) {
    $page = $file.FullName.Substring($root.Length).TrimStart('\', '/').Replace('\', '/')
    $body = [System.IO.File]::ReadAllText($file.FullName)
    $pages++
    foreach ($m in $block.Matches($body)) {
        $preAttributes = $m.Groups[1].Value
        $codeAttributes = $m.Groups[2].Value
        $inner = $m.Groups[3].Value
        $found = $langOf.Match($codeAttributes)
        if (-not $found.Success) { $found = $langClass.Match($codeAttributes) }
        $lang = if ($found.Success) { $found.Groups[1].Value } else { '(none)' }
        $census[$lang] = 1 + $(if ($census.ContainsKey($lang)) { $census[$lang] } else { 0 })
        if ($preAttributes -notmatch 'chroma') {
            if ($lang -ne 'text') {
                $first = @((Get-PlainText $inner).Trim() -split "`n")[0]
                $plain.Add("NOT HIGHLIGHTED $page [$lang] $first")
            }
            continue
        }
        # A token belongs to the line whose span starts last before it.
        $starts = @($lineStart.Matches($inner) | ForEach-Object Index) + $inner.Length
        foreach ($e in $errToken.Matches($inner)) {
            $k = 0
            for ($j = 0; $j -lt $starts.Count - 1; $j++) { if ($starts[$j] -le $e.Index) { $k = $j } }
            $line = (Get-PlainText $inner.Substring($starts[$k], $starts[$k + 1] - $starts[$k])).Trim()
            $errors.Add("ERROR TOKEN $page [$lang] '$(Get-PlainText $e.Groups[1].Value)' in: $line")
        }
    }
}
"pages: $pages"
foreach ($lang in ($census.Keys | Sort-Object { -$census[$_] }, { $_ })) { '  {0,-12} {1,5} blocks' -f $lang, $census[$lang] }
foreach ($line in $errors) { $line }
foreach ($line in $plain) { $line }
"ERRORS $($errors.Count) PLAIN $($plain.Count)"
exit ($errors.Count + $plain.Count)
