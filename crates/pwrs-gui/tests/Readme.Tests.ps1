# README.md against what it describes. Every count it states, and every
# command its table names, is checked against the source: the module's
# commands, the wiki's reference index and pages, the patches the
# workspace applies, and its manifests. A change that adds a command, a
# page or a patch changes the README in the same commit. Each phrase the
# suite reads must appear exactly once, so rewording one changes this
# suite with it.

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-Module $script:GuiModule
    $script:Root = Split-Path (Split-Path (Split-Path $PSScriptRoot))
    $script:Readme = Get-Content -Raw -LiteralPath (Join-Path $script:Root 'README.md')
    $script:Docs = Join-Path (Join-Path (Join-Path $script:Root 'wiki') 'content') 'docs'
    $script:Commands = @(Get-Command -Module PWRS-GUI | ForEach-Object Name | Sort-Object)

    # The number a count in the README stands for, in digits or in words,
    # such as 86, eleven or twenty-one.
    function ConvertFrom-Count([string] $Text) {
        if ($Text -match '^\d+$') { return [int] $Text }
        $words = @{
            one = 1; two = 2; three = 3; four = 4; five = 5; six = 6; seven = 7; eight = 8; nine = 9
            ten = 10; eleven = 11; twelve = 12; thirteen = 13; fourteen = 14; fifteen = 15; sixteen = 16
            seventeen = 17; eighteen = 18; nineteen = 19; twenty = 20; thirty = 30; forty = 40; fifty = 50
            sixty = 60; seventy = 70; eighty = 80; ninety = 90
        }
        $sum = 0
        foreach ($part in $Text.ToLowerInvariant() -split '-') {
            if (-not $words.ContainsKey($part)) { throw "'$Text' is not a count this suite reads" }
            $sum += $words[$part]
        }
        $sum
    }

    # The groups of the README's one match for $Pattern, in order, as an
    # array even when there is one.
    function Get-ReadmeMatch([string] $Pattern) {
        $found = [regex]::Matches($script:Readme, $Pattern)
        if ($found.Count -ne 1) { throw "the README matches '$Pattern' $($found.Count) times, where this suite reads it once" }
        , @($found[0].Groups | Select-Object -Skip 1 | ForEach-Object Value)
    }

    # The names in a list written as "a, b, c and d".
    function Split-NameList([string] $List) {
        @($List -split ',\s*|\s+and\s+' | ForEach-Object Trim | Where-Object { $_ } | Sort-Object)
    }

    # How many pages a section of the wiki has, its index left out.
    function Get-PageCount([string] $Section) {
        @(Get-ChildItem -LiteralPath (Join-Path $script:Docs $Section) -File -Filter *.md | Where-Object Name -ne '_index.md').Count
    }

    # The reference index's sections, keyed by the anchor Hugo gives each
    # heading, each with the pages its cards link. A card's title can be a
    # widget's short name, so its link names the page, and a command's page
    # is named after the command.
    $reference = Join-Path $script:Docs 'reference'
    $pages = @{}
    foreach ($page in Get-ChildItem -LiteralPath $reference -File -Filter *.md) { $pages[$page.BaseName.ToLowerInvariant()] = $page.BaseName }
    $script:Sections = @{}
    $section = $null
    foreach ($line in Get-Content -LiteralPath (Join-Path $reference '_index.md')) {
        if ($line -match '^## (.+)$') {
            $section = ($Matches[1].ToLowerInvariant() -replace '[^a-z0-9 -]', '') -replace ' ', '-'
            $script:Sections[$section] = [System.Collections.Generic.List[string]]::new()
        } elseif ($section -and $line -match '\{\{<\s*card\s.*\blink="([^"/]+)/?"') {
            $slug = $Matches[1].ToLowerInvariant()
            $name = if ($pages.ContainsKey($slug)) { $pages[$slug] } else { "a card linking $slug/, which is no page" }
            $script:Sections[$section].Add($name)
        }
    }
}

Describe 'README.md' {
    It "counts the widgets the reference's index lists" {
        $count = (Get-ReadmeMatch '\[(\w+) widgets\]')[0]
        ConvertFrom-Count $count | Should -Be $script:Sections['widgets'].Count
    }

    It "counts the module's commands, in both places" {
        ConvertFrom-Count (Get-ReadmeMatch 'its (\w+) commands')[0] | Should -Be $script:Commands.Count
        ConvertFrom-Count (Get-ReadmeMatch 'one for each of the (\w+) commands')[0] | Should -Be $script:Commands.Count
    }

    It 'names every command in its table, by name or in a group row with its count' {
        $table = ($script:Readme -split '(?m)^## ' | Where-Object { $_ -like "Commands*" })
        $named = [System.Collections.Generic.List[string]]::new()
        foreach ($row in $table -split '\r?\n' | Where-Object { $_ -like '| `[*' }) {
            if ($row -match '^\| \[`([A-Za-z]+-Gui[A-Za-z]+)`\]') {
                $named.Add($Matches[1])
            } elseif ($row -match '^\| \[[^\]]+\]\([^)#]*#([a-z0-9-]+)\), (\w+) of them') {
                $anchor = $Matches[1]
                $cards = $script:Sections[$anchor]
                $cards | Should -Not -BeNullOrEmpty -Because "the row naming #$anchor names a section of the reference's index"
                ConvertFrom-Count $Matches[2] | Should -Be $cards.Count -Because "the row naming #$anchor counts that section's cards"
                $named.AddRange($cards)
            } else {
                throw "a row of the Commands table names neither a command nor a group of them: $row"
            }
        }
        $listed = @($named | Sort-Object -Unique)
        @($script:Commands | Where-Object { $listed -notcontains $_ }) -join ', ' | Should -BeNullOrEmpty -Because 'every command has a row, or a group row that covers it'
        @($listed | Where-Object { $script:Commands -notcontains $_ }) -join ', ' | Should -BeNullOrEmpty -Because 'every name the table gives is a command'
    }

    It "counts the wiki's pages, section by section" {
        $counts = Get-ReadmeMatch 'a tutorial in (\w+) chapters, (\w+) how-to guides, (\w+) explanations, (\w+) reference pages'
        ConvertFrom-Count $counts[0] | Should -Be (Get-PageCount 'tutorials') -Because 'each page of the tutorial is a chapter'
        ConvertFrom-Count $counts[1] | Should -Be (Get-PageCount 'how-to')
        ConvertFrom-Count $counts[2] | Should -Be (Get-PageCount 'explanation')
        ConvertFrom-Count $counts[3] | Should -Be (Get-PageCount 'reference')
        ConvertFrom-Count (Get-ReadmeMatch 'builds over (\w+) chapters')[0] | Should -Be (Get-PageCount 'tutorials')
        ConvertFrom-Count (Get-ReadmeMatch 'and (\w+) pages on building from source')[0] | Should -Be (Get-PageCount 'building')
    }

    It 'names the crates the workspace patches, in both places' {
        $patched = @(Get-ChildItem -LiteralPath (Join-Path $script:Root 'patches') -File -Filter *.patch |
            ForEach-Object { $_.Name -replace '-\d+(\.\d+)*\.patch$', '' } | Sort-Object)
        foreach ($pattern in 'unpacks the (\w+) crates the workspace patches, (.+?), from crates\.io', 'applies to (\w+) crates\.io crates, (.+?), and `apply\.ps1`') {
            $found = Get-ReadmeMatch $pattern
            ConvertFrom-Count $found[0] | Should -Be $patched.Count
            (Split-NameList $found[1]) -join ', ' | Should -Be ($patched -join ', ')
        }
    }

    It "counts the module's helper executables" {
        $manifest = Get-Content -Raw -LiteralPath (Join-Path (Join-Path (Join-Path $script:Root 'crates') 'pwrs-gui') 'Cargo.toml')
        ConvertFrom-Count (Get-ReadmeMatch 'and the (\w+) helper executables')[0] | Should -Be ([regex]::Matches($manifest, '(?m)^\[\[bin\]\]')).Count
    }

    It 'counts and names the source trees beside the checkout' {
        $manifest = Get-Content -Raw -LiteralPath (Join-Path $script:Root 'Cargo.toml')
        $trees = @([regex]::Matches($manifest, 'path\s*=\s*"\.\./([^/"]+)') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
        ConvertFrom-Count (Get-ReadmeMatch 'builds on (\w+) source trees beside the checkout')[0] | Should -Be $trees.Count
        foreach ($tree in $trees) { $script:Readme | Should -Match ([regex]::Escape("``../$tree``")) }
    }
}
