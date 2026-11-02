# Writes the wiki's generated reference pages from the built module:
#
#   wiki/content/docs/reference/Error-Reference.md    every error id, its category and its messages
#   wiki/content/docs/reference/powershell/<Group>-Cmdlets.md
#                                                     the syntax, parameters and errors of a group of cmdlets
#   wiki/content/docs/reference/powershell/Types.md   every class and enum, its members, its values
#                                                     and the errors its methods throw
#   wiki/content/docs/reference/powershell/_index.md  every cmdlet, after the page's hand-written head
#
#   pwsh -NoProfile -File tools/Export-Reference.ps1 -Module target/pwrs/Wgpu
#
# Syntax, parameter sets, positions, pipeline input and types are read from
# the imported module by reflection. A cmdlet's text and each parameter's
# come from the help the module ships, which PWRS generates from the doc
# comments in src/. A class's text, each member's and each enum value's
# come from the doc comment in src/ it was generated from. The errors each
# cmdlet raises and each class's methods throw are written by hand in
# tools/Error-Lists.psd1. The Error Reference keeps what it holds from its
# `## Examples` heading on: that part is written by hand, and
# tools/Invoke-Documentation.ps1 writes the output its examples show. The
# powershell/ pages hold no examples: each group page names the task pages
# that do, and the index keeps what it holds above its `## Every cmdlet`
# heading. Every cmdlet must have a group page and a list of errors, and
# every cmdlet and class a list names must be one the module exports, or
# nothing is written. The error ids, their categories and their messages
# are read from the calls in src/ that raise them; an id named anywhere
# else in src/, outside the tests, or a list naming an id that none of
# them raises, stops the run.
[CmdletBinding()]
param(
    # The built module: its folder, or the Wgpu.psd1 inside it.
    [Parameter(Mandatory)] [string] $Module,
    # The wiki folder; the pages are written under content/docs/reference in it.
    [string] $WikiPath = (Join-Path (Join-Path $PSScriptRoot '..') 'wiki')
)

$ErrorActionPreference = 'Stop'
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$item = Get-Item -LiteralPath $Module
$manifest = if ($item.PSIsContainer) { Join-Path $item.FullName 'Wgpu.psd1' } else { $item.FullName }
Import-Module $manifest -Force

$reference = Join-Path $WikiPath 'content/docs/reference'

# ---------------------------------------------------------------------
# The module's surface.
# ---------------------------------------------------------------------

$cmdlets = @(Get-Command -Module Wgpu -CommandType Cmdlet | Sort-Object Name)
if ($cmdlets.Count -eq 0) { throw "the module at $manifest exports no cmdlets" }
$aliasesOf = @{}
foreach ($a in Get-Command -Module Wgpu -CommandType Alias) { $aliasesOf[$a.Definition] += @($a.Name) }
$types = @([AppDomain]::CurrentDomain.GetAssemblies() |
    Where-Object { $_.GetName().Name -like 'Wgpu.Shell*' } |
    ForEach-Object { $_.GetExportedTypes() } |
    Where-Object { $_.Namespace -eq 'Wgpu' })
$classes = @{}
foreach ($t in $types) {
    if ($t.IsClass -and -not $t.IsAbstract -and -not $t.IsSubclassOf([System.Management.Automation.PSCmdlet])) { $classes[$t.FullName] = $t }
}
$enums = @($types | Where-Object IsEnum | Sort-Object Name)

# The cmdlets of each powershell/ group page, in page order, with the
# page's title and the task pages that hold their examples.
$SurfacePages = [ordered]@{
    'Device-Cmdlets'  = @{ Title = 'Adapters and devices'; Tasks = @('Devices'); Cmdlets = @('Get-WgpuAdapter', 'Open-WgpuDevice', 'Get-WgpuDevice', 'Wait-WgpuDevice', 'Close-WgpuDevice') }
    'Buffer-Cmdlets'  = @{ Title = 'Buffers'; Tasks = @('Buffers'); Cmdlets = @('New-WgpuBuffer', 'Write-WgpuBuffer', 'Read-WgpuBuffer', 'Copy-WgpuBuffer', 'Clear-WgpuBuffer', 'Remove-WgpuBuffer') }
    'Compute-Cmdlets' = @{ Title = 'Compute'; Tasks = @('Compute'); Cmdlets = @('Invoke-WgpuCompute', 'Invoke-WgpuMap', 'Measure-WgpuArray', 'New-WgpuComputePipeline', 'New-WgpuBindGroup') }
    'Shader-Cmdlets'  = @{ Title = 'Shaders'; Tasks = @('Shaders'); Cmdlets = @('New-WgpuShader', 'Test-WgpuShader', 'Get-WgpuShaderInfo', 'ConvertTo-WgpuShader') }
    'Texture-Cmdlets' = @{ Title = 'Textures and images'; Tasks = @('Images'); Cmdlets = @('New-WgpuTexture', 'Write-WgpuTexture', 'Read-WgpuTexture', 'Import-WgpuImage', 'Export-WgpuImage', 'Resize-WgpuTexture', 'Convert-WgpuTexture', 'ConvertTo-WgpuNormalMap', 'ConvertTo-WgpuCubeMap', 'ConvertFrom-WgpuCubeMap', 'ConvertTo-WgpuIrradianceMap', 'ConvertTo-WgpuSpecularMap', 'New-WgpuBrdfTable', 'Join-WgpuTexture', 'Compare-WgpuTexture', 'Measure-WgpuTexture', 'Get-WgpuTextureFormat', 'New-WgpuSampler') }
    'Render-Cmdlets'  = @{ Title = 'Rendering'; Tasks = @('Rendering'); Cmdlets = @('Invoke-WgpuRender', 'New-WgpuRenderPipeline') }
    'Command-Cmdlets' = @{ Title = 'Recording commands'; Tasks = @('Recording'); Cmdlets = @('New-WgpuCommandEncoder', 'Submit-WgpuCommand', 'New-WgpuQuerySet') }
}
$surfaceOf = @{}
foreach ($pageName in $SurfacePages.Keys) {
    foreach ($name in $SurfacePages[$pageName].Cmdlets) {
        if ($surfaceOf.ContainsKey($name)) { throw "`$SurfacePages puts $name on $($surfaceOf[$name]) and on $pageName" }
        $surfaceOf[$name] = $pageName
    }
}
$unplaced = @($cmdlets | Where-Object { -not $surfaceOf.ContainsKey($_.Name) } | ForEach-Object Name)
if ($unplaced.Count) { throw "cmdlets with no page in `$SurfacePages: $($unplaced -join ', ')" }
$strays = @($surfaceOf.Keys | Where-Object { $_ -notin $cmdlets.Name } | Sort-Object)
if ($strays.Count) { throw "`$SurfacePages names cmdlets the module does not export: $($strays -join ', ')" }

# The errors each cmdlet raises, and those each class's methods throw,
# written by hand.
$ErrorLists = Import-PowerShellDataFile -LiteralPath (Join-Path $PSScriptRoot 'Error-Lists.psd1')
$listless = @($cmdlets.Name | Where-Object { -not $ErrorLists.Cmdlets.ContainsKey($_) })
if ($listless.Count) { throw "tools/Error-Lists.psd1 lists no errors for $($listless -join ', ')" }
$foreign = @(@($ErrorLists.Cmdlets.Keys | Where-Object { $_ -notin $cmdlets.Name }) + @($ErrorLists.Classes.Keys | Where-Object { -not $classes.ContainsKey($_) }) | Sort-Object)
if ($foreign.Count) { throw "tools/Error-Lists.psd1 lists the errors of what the module does not export: $($foreign -join ', ')" }

# ---------------------------------------------------------------------
# Doc comments, as paragraphs, keyed 'Wgpu.Device' for a type and
# 'Wgpu.Device.Upload' for a member or an enum value; the source file of
# each class, enum and cmdlet.
# ---------------------------------------------------------------------

$Docs = @{}
$SourceOf = @{}

function ConvertTo-PascalCase([string] $Snake) {
    -join ($Snake.Split('_') | Where-Object { $_ } |
        ForEach-Object { $_.Substring(0, 1).ToUpperInvariant() + $_.Substring(1) })
}

# The paragraphs of a doc comment, split at its blank lines.
function Split-DocLine([string[]] $Lines) {
    $paragraphs = [System.Collections.Generic.List[string]]::new()
    $current = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $Lines) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            if ($current.Count) { $paragraphs.Add($current -join ' '); $current.Clear() }
            continue
        }
        $current.Add($line.Trim())
    }
    if ($current.Count) { $paragraphs.Add($current -join ' ') }
    , $paragraphs.ToArray()
}

# rustfmt puts an item's doc comment and attributes at the margin, and a
# field, a variant, a method and their doc comments four spaces in, with
# the closing brace of the struct, enum or impl at the margin.
foreach ($file in Get-ChildItem -LiteralPath (Join-Path $repo 'src') -Recurse -Filter '*.rs') {
    $relative = [System.IO.Path]::GetRelativePath($repo, $file.FullName).Replace('\', '/')
    $doc = [System.Collections.Generic.List[string]]::new()
    $pending = $null
    $pendingDoc = @()
    $methodsNext = $false
    $inStruct = $null
    $inEnum = $null
    $inImpl = $null
    $skipField = $false
    foreach ($line in [System.IO.File]::ReadAllLines($file.FullName)) {
        if ($inStruct -or $inEnum -or $inImpl) {
            if ($line -match '^}') { $inStruct = $null; $inEnum = $null; $inImpl = $null; $doc.Clear(); continue }
            if ($line -match '^    ///\s?(.*)$') { $doc.Add($Matches[1]); continue }
            if ($line -match '^    //') { continue }
            if ($line -match '^    #\[psfield\(skip') { $skipField = $true; continue }
            if ($line -match '^    #\[') { continue }
            if ($inStruct -and $line -match '^    pub (\w+)\s*:') {
                if (-not $skipField) { $Docs["$inStruct.$(ConvertTo-PascalCase $Matches[1])"] = Split-DocLine $doc.ToArray() }
                $skipField = $false
                $doc.Clear()
                continue
            }
            if ($inEnum -and $line -match '^    (\w+)\s*(=[^,]*)?,?\s*$') {
                $Docs["$inEnum.$($Matches[1])"] = Split-DocLine $doc.ToArray()
                $doc.Clear()
                continue
            }
            if ($inImpl -and $line -match '^    pub fn (\w+)') {
                $name = if ($Matches[1] -eq 'new') { '.ctor' } else { ConvertTo-PascalCase $Matches[1] }
                $Docs["$inImpl.$name"] = Split-DocLine $doc.ToArray()
                $doc.Clear()
                continue
            }
            if ($line.Trim()) { $skipField = $false; $doc.Clear() }
            continue
        }
        if ($line -match '^///\s?(.*)$') { $doc.Add($Matches[1]); continue }
        if ($line -match '^#\[(?:cfg_attr\([^,]+,\s*)?(psclass|psenum)\(.*name\s*=\s*"([^"]+)"') {
            $pending = $Matches[2]
            $pendingDoc = $doc.ToArray()
            continue
        }
        if ($line -match '^#\[cmdlet\(verb\s*=\s*"(\w+)",\s*noun\s*=\s*"(\w+)"') {
            $SourceOf["$($Matches[1])-$($Matches[2])"] = $relative
            $doc.Clear()
            continue
        }
        if ($line -match '^#\[psmethods\]') { $methodsNext = $true; continue }
        if ($line -match '^#\[') { continue }
        if ($pending -and $line -match '^pub (struct|enum) (\w+)') {
            $Docs[$pending] = Split-DocLine $pendingDoc
            $SourceOf[$pending] = $relative
            $SourceOf["struct:$($Matches[2])"] = $pending
            if ($line -notmatch ';\s*$' -and $line -notmatch '\{\}\s*$') {
                if ($Matches[1] -eq 'struct') { $inStruct = $pending } else { $inEnum = $pending }
            }
            $pending = $null
            $doc.Clear()
            continue
        }
        if ($methodsNext -and $line -match '^impl (\w+)') {
            $inImpl = $SourceOf["struct:$($Matches[1])"]
            if (-not $inImpl) { throw "${relative}: #[psmethods] impl $($Matches[1]) has no #[psclass] above it in this file" }
            $methodsNext = $false
            $doc.Clear()
            continue
        }
        if ($line.Trim()) { $doc.Clear() }
    }
}

# ---------------------------------------------------------------------
# Formatting.
# ---------------------------------------------------------------------

function Get-InnerType([type] $Type) {
    $inner = $Type
    while ($inner -and ($inner.IsArray -or ($inner.IsGenericType -and $inner.GetGenericTypeDefinition() -eq [System.Nullable``1]))) {
        $inner = if ($inner.IsArray) { $inner.GetElementType() } else { $inner.GetGenericArguments()[0] }
    }
    $inner
}

# A CLR type as it reads in the shell: Wgpu types in full, a Nullable as the
# type with a question mark (left off a parameter, whose table says whether
# it is required), a task with what it completes with, and the rest by
# PowerShell's short name.
function Format-Type([type] $Type, [switch] $Parameter) {
    if ($null -eq $Type) { return '' }
    if ($Type.IsGenericType -and $Type.GetGenericTypeDefinition() -eq [System.Nullable``1]) {
        $inner = Format-Type $Type.GetGenericArguments()[0]
        if ($Parameter) { return $inner }
        return "${inner}?"
    }
    if ($Type.IsArray) { return (Format-Type $Type.GetElementType()) + '[]' }
    if ($Type.IsGenericType -and $Type.Name -eq 'Task`1') { return "Task[$(Format-Type $Type.GetGenericArguments()[0])]" }
    switch ($Type.FullName) {
        'System.String' { return 'string' }
        'System.Boolean' { return 'bool' }
        'System.Byte' { return 'byte' }
        'System.UInt16' { return 'ushort' }
        'System.Int32' { return 'int' }
        'System.UInt32' { return 'uint' }
        'System.Int64' { return 'long' }
        'System.UInt64' { return 'ulong' }
        'System.Single' { return 'float' }
        'System.Double' { return 'double' }
        'System.Object' { return 'object' }
        'System.Void' { return 'void' }
        'System.Array' { return 'array' }
        'System.Collections.Hashtable' { return 'hashtable' }
        'System.Management.Automation.SwitchParameter' { return 'switch' }
        'System.Threading.CancellationToken' { return 'CancellationToken' }
        'System.Threading.Tasks.Task' { return 'Task' }
    }
    if ($Type.Namespace -eq 'Wgpu') { return $Type.FullName }
    $Type.Name
}

# The heading id Hugo gives a heading of one word.
function Get-HeadingId([string] $Heading) { $Heading.ToLowerInvariant() -replace '[^a-z0-9_-]', '' }

# Prose from help or a doc comment, with the angle brackets outside code
# spans escaped so markdown shows them, and a rustdoc link, [`name`], left
# as the code span it shows.
function Format-Prose([string] $Text) {
    if ([string]::IsNullOrWhiteSpace($Text)) { return '' }
    $Text = $Text -replace '\[(`[^`]+`)\](?![(\[])', '$1'
    $parts = foreach ($part in ($Text -split '(`[^`]*`)')) {
        if ($part.Length -ge 2 -and $part.StartsWith('`') -and $part.EndsWith('`')) { $part }
        else { $part -replace '<', '&lt;' -replace '>', '&gt;' }
    }
    (-join $parts).Trim()
}

function Format-Cell([string] $Text) { (Format-Prose ($Text -replace '\r?\n', ' ')) -replace '\|', '\|' }

# Names as code spans, joined with commas.
function Format-CodeList([string[]] $Names) {
    $tick = [char]0x60
    (@($Names | Where-Object { $_ } | ForEach-Object { $tick + $_ + $tick })) -join ', '
}

# The help's paragraphs: a paragraph break of the doc comment it was
# generated from arrives as a blank line or a run of spaces.
function Split-HelpText([string] $Text) {
    , @(($Text -split '\r?\n\s*\r?\n|(?<=\S)\s{2,}(?=\S)') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

function Get-HelpText($Field) {
    if ($null -eq $Field) { return '' }
    (@($Field | ForEach-Object { $_.Text }) -join "`n`n").Trim()
}

# The front matter a page begins with.
function Get-FrontMatter([string] $Title, [int] $Weight) {
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('---')
    $lines.Add("title: $Title")
    $lines.Add("weight: $Weight")
    $lines.Add('---')
    $lines.Add('')
    , $lines
}

# The lines of an existing page from its `## Examples` heading on, or none.
function Get-HandWrittenTail([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return , @() }
    $lines = [System.IO.File]::ReadAllLines($Path)
    $at = [array]::IndexOf($lines, '## Examples')
    if ($at -lt 0) { return , @() }
    , [string[]]$lines[$at..($lines.Count - 1)]
}

function Write-Page([string] $Path, [System.Collections.Generic.List[string]] $Lines) {
    while ($Lines.Count -and $Lines[$Lines.Count - 1] -eq '') { $Lines.RemoveAt($Lines.Count - 1) }
    [System.IO.File]::WriteAllText($Path, ($Lines -join "`n") + "`n", [System.Text.UTF8Encoding]::new($false))
}

# The lines of an existing page before its $Heading line, or $null for a
# page not yet written.
function Get-HandWrittenHead([string] $Path, [string] $Heading) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $lines = [System.IO.File]::ReadAllLines($Path)
    $at = [array]::IndexOf($lines, $Heading)
    if ($at -lt 0) { throw "$Path has no line '$Heading', where its generated part begins" }
    $head = [System.Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $at; $i++) { $head.Add($lines[$i]) }
    , $head
}

# Words joined as a sentence joins them: `a`, `a and b`, `a, b and c`, or
# with `or` as $Conjunction.
function Join-Word([string[]] $Words, [string] $Conjunction = 'and') {
    if ($Words.Count -le 1) { return ($Words -join '') }
    ($Words[0..($Words.Count - 2)] -join ', ') + " $Conjunction " + $Words[-1]
}

# A type as the powershell/ pages show it: a class or enum of the module
# linked to its section of Types.md, written as $Page for a link from
# another page and as '' for one on Types.md itself; any other type by its
# short name.
function Format-SurfaceType([type] $Type, [string] $Page, [switch] $Parameter) {
    $text = Format-Type $Type -Parameter:$Parameter
    $inner = Get-InnerType $Type
    if ($inner -and $inner.Namespace -eq 'Wgpu' -and ($classes.ContainsKey($inner.FullName) -or $inner.IsEnum) -and $text.StartsWith($inner.FullName)) {
        return "[$($inner.FullName)]($Page#$(Get-HeadingId $inner.FullName))$($text.Substring($inner.FullName.Length))"
    }
    $text
}

# A type a cmdlet's OutputType names, which may be a name only, as the
# sentence that ends a cmdlet's section shows it.
function Format-SurfaceOutput($OutputType) {
    if ($OutputType.Type) {
        $shown = Format-SurfaceType $OutputType.Type 'Types.md'
        if ($shown.StartsWith('[')) { return $shown }
        return "``$shown``"
    }
    $name = $OutputType.Name
    if ($classes.ContainsKey($name)) { return "[$name](Types.md#$(Get-HeadingId $name))" }
    "``$name``"
}

$common = [System.Management.Automation.PSCmdlet]::CommonParameters + [System.Management.Automation.PSCmdlet]::OptionalCommonParameters

# The cmdlets that write each class.
$writers = @{}
foreach ($c in $cmdlets) {
    foreach ($ot in @($c.OutputType)) {
        $name = if ($ot.Type) { $ot.Type.FullName } else { $ot.Name }
        $writers[$name] += @($c.Name)
    }
}

function Get-DocParagraph([string] $Key) { @($Docs[$Key] | Where-Object { $_ }) }

# ---------------------------------------------------------------------
# The error reference.
# ---------------------------------------------------------------------

# What a value filled into a message stands for, where the name the source
# gives it would not tell a reader: for one id, as 'Id:expression', then
# for any, by the whole expression and then by its last name.
$PlaceholderAt = @{
    'WgpuAdjust:name'                      = '-Brightness or -Contrast'
    'WgpuAspect:role'                      = 'the reference or the texture'
    'WgpuColorSpace:own'                   = 'format'
    'WgpuCompare:shared'                   = 'channels'
    'WgpuCompare:what'                     = 'depth or number of layers'
    'WgpuBindingMissing:names'             = 'bindings'
    'WgpuClearDepth:depth_name'            = '-ClearDepth or clear_depth'
    'WgpuClearDepth:lacking(format, "depth")' = 'what the depth target lacks'
    'WgpuClearStencil:stencil_name'        = '-ClearStencil or clear_stencil'
    'WgpuClearStencil:lacking(format, "stencil")' = 'what the depth target lacks'
    'WgpuDepthFormat:name'                 = 'format'
    'WgpuDepthFormat:names'                = 'the depth and stencil formats'
    'WgpuDepthTest:lacking(format, "depth")' = 'what the depth target lacks'
    'WgpuStencil:lacking(format, "stencil")' = 'what the depth target lacks'
    'WgpuCompress:target'                  = 'format'
    'WgpuCompressFormat:other'             = 'format'
    'WgpuConvert:into'                     = 'format'
    'WgpuCopyAlignment:what'               = 'offset or size'
    'WgpuCrop:height'                      = 'texture height'
    'WgpuCrop:width'                       = 'texture width'
    'WgpuCubeMap:holds'                    = 'what it holds'
    'WgpuCubeMap:name'                     = 'face'
    'WgpuCubeMap:names[0]'                 = 'the first face'
    'WgpuCubeMap:needs'                    = 'cells across and down, or square'
    'WgpuCubeMap:shape'                    = 'its layers or dimension'
    'WgpuCubeMap:shape(lead)'              = 'its size and format'
    'WgpuCubeMap:shape(t)'                 = 'size and format'
    'WgpuCubeMap:SHAPES'                   = 'the aspect ratio of each shape'
    'WgpuCubeMap:what'                     = 'what is made'
    'WgpuCubeMap:y'                        = 'value'
    'WgpuDecoder:from'                     = 'format'
    'WgpuDispatchSize:what'                = 'parameter'
    'WgpuEffort:target'                    = 'format'
    'WgpuGrayscale:name'                   = 'text'
    'WgpuImageFormat:from'                 = 'format'
    'WgpuImageFormat:into'                 = 'decoded format'
    'WgpuImageFormat:other'                = 'format'
    'WgpuImageShape:what'                  = 'color type'
    'WgpuJoin:label'                       = 'texture'
    'WgpuJoin:name'                        = 'parameter'
    'WgpuJoin:what'                        = 'what was given'
    'WgpuKtx2:written'                     = 'n'
    'WgpuKtx2:zstd_error(written)'         = 'reason'
    'WgpuMipLevels:full'                   = 'levels'
    'WgpuMipLevels:texture.mip_level_count() - 1' = 'last level'
    'WgpuNoWgslType:what'                  = 'variable'
    'WgpuNormalMap:b.name'                 = 'format'
    'WgpuNormalMap:f'                      = 'format'
    'WgpuNormalMap:f.remove_srgb_suffix()' = 'linear format'
    'WgpuNormalMap:what'                   = 'what the step needs'
    'WgpuNormalMap:which'                  = 'smallest or largest'
    'WgpuOutFile:target'                   = 'path'
    'WgpuPad:ph'                           = 'height'
    'WgpuPad:pw'                           = 'width'
    'WgpuPad:resized.0'                    = 'texture width'
    'WgpuPad:resized.1'                    = 'texture height'
    'WgpuReduction:other'                  = 'outcome'
    'WgpuResize:v'                         = 'side'
    'WgpuRotate:other'                     = 'degrees'
    'WgpuSsim:limit'                       = 'n'
    'WgpuTexels:other'                     = 'format'
    'WgpuTexture:what'                     = 'multisampled or depth'
    'WgpuTextureDecode:into'               = 'format'
    'WgpuTextureDecode:other'              = 'format'
    'WgpuTextureDecode:slice_len * slices' = 'size'
    'WgpuTextureFile:dds.data.len()'       = 'size'
    'WgpuTextureFile:other'                = 'scheme'
    'WgpuTextureFormat:missing'            = 'features'
    'WgpuTextureRead:read_as'              = 'format'
    'WgpuTextureUsage:missing'             = 'usages'
    'WgpuTextureUsage:names'               = 'usages'
    'WgpuTextureUsage:role'                = 'the reference or the texture'
    'WgpuTextureUsage:what'                = 'what the call does'
    'WgpuTextureUsage:why'                 = 'what it needs them for'
    'WgpuTransform:change'                 = 'a crop to, a resize to or a canvas of'
    'WgpuTransform:decoded'                = 'format'
    'WgpuTransform:from'                   = 'format'
    'WgpuTranslate:target'                 = 'language'
    'WgpuVersion:what'                     = 'language'
}
$PlaceholderOf = @{
    'adapter.get_info().name'         = 'adapter'
    'core.info.name'                  = 'adapter'
    'expected(binding.kind)'          = 'resource type'
    'member.numbers()'                = 'n'
    'mpsc::RecvError'                 = 'reason'
    'other.label'                     = 'device'
    'printed(&lines)'                 = 'what Basis Universal printed'
    'self.core.label'                 = 'device'
    'std::io::Error::last_os_error()' = 'reason'
    'wgpu::BufferAsyncError'          = 'reason'
    'b'                               = 'binding'
    'e'                               = 'reason'
    'file'                            = 'path'
    'g'                               = 'group'
    'given'                           = 'n'
    'h'                               = 'height'
    'listed'                          = 'adapters'
    'missing'                         = 'bindings'
    'other'                           = 'type'
    'payload'                         = 'message'
    'taken'                           = 'errors'
    'ty'                              = 'type'
    'v'                               = 'value'
    'w'                               = 'width'
    'wanted'                          = 'text'
    'why'                             = 'reason'
}
# The messages of an id whose raising call takes its message from a variable.
$MessagesOf = @{
    'WgpuShaderInvalid' = @('the shader is invalid: line <line>, column <column>: <message>', 'the shader is invalid: <message>')
}

# The text of a Rust string literal's body.
function ConvertFrom-RustString([string] $Body) {
    $Body = $Body -replace '\\\r?\n\s*', ''
    [regex]::Replace($Body, '\\(.)', {
            param($m)
            switch -CaseSensitive ($m.Groups[1].Value) { 'n' { "`n" } 't' { "`t" } default { $m.Groups[1].Value } }
        })
}

# The arguments of a call from $At, inside its opening parenthesis, to its
# closing one, split at the commas between them.
function Get-RustArgument([string] $Text, [int] $At) {
    $parts = [System.Collections.Generic.List[string]]::new()
    $depth = 0
    $start = $At
    for ($i = $At; $i -lt $Text.Length; $i++) {
        $c = $Text[$i]
        if ($c -eq '"') {
            for ($i++; $i -lt $Text.Length -and $Text[$i] -ne '"'; $i++) { if ($Text[$i] -eq '\') { $i++ } }
        } elseif ($c -eq "'") {
            # A char literal, as ',' or '\n'; a lifetime has no closing quote.
            if ($i + 2 -lt $Text.Length -and $Text[$i + 2] -eq "'") { $i += 2 }
            elseif ($i + 3 -lt $Text.Length -and $Text[$i + 1] -eq '\' -and $Text[$i + 3] -eq "'") { $i += 3 }
        } elseif ('([{'.Contains($c)) {
            $depth++
        } elseif (')]}'.Contains($c)) {
            if ($depth -eq 0) {
                $parts.Add($Text.Substring($start, $i - $start).Trim())
                break
            }
            $depth--
        } elseif ($c -eq ',' -and $depth -eq 0) {
            $parts.Add($Text.Substring($start, $i - $start).Trim())
            $start = $i + 1
        }
    }
    , [string[]]@($parts | Where-Object { $_ })
}

# What the value $Expr stands for in a message raised as $Id.
function Get-Placeholder([string] $Expr, [string] $Id) {
    $e = $Expr.Trim().TrimStart('&').Trim()
    while ($true) {
        if ($PlaceholderAt.ContainsKey("${Id}:$e")) { return $PlaceholderAt["${Id}:$e"] }
        if ($PlaceholderOf.ContainsKey($e)) { return $PlaceholderOf[$e] }
        if ($e -match '\.len\(\)$') { return 'n' }
        if ($e -match '^(.+?)\.iter\(\)') { $e = $Matches[1]; continue }
        if ($e -match '^(.+)\.(display|join|to_string|as_str)\([^()]*\)$') { $e = $Matches[1]; continue }
        if ($e -match '^[\w:]+\((.+)\)$') { $e = $Matches[1].Trim().TrimStart('&').Trim(); continue }
        if ($e -match '^\w+(\.\w+)+$') { $e = ($e -split '\.')[-1]; continue }
        if ($e -match '^\w+(?:\.\w+)*\.(\w+)\(\)$') { $e = $Matches[1]; continue }
        break
    }
    if ($e -notmatch '^\w+( [-+*] \w+)*$') {
        throw "a message of $Id fills in $Expr, which the reference cannot name; add it to `$PlaceholderAt or `$PlaceholderOf"
    }
    $e -replace '_', ' '
}

# A message argument as the text it makes, each value it fills in written
# as <what it stands for>, or $null for a message held in a variable.
function ConvertTo-MessageTemplate([string] $Expr, [string] $Id) {
    $e = $Expr.Trim().TrimStart('&').Trim() -replace '\.(to_string|to_owned|into)\(\)$', ''
    if ($e -match '(?s)^"((?:[^"\\]|\\.)*)"$') { return ConvertFrom-RustString $Matches[1] }
    if ($e -notmatch '^format!\(') { return $null }
    $parts = Get-RustArgument $e ($e.IndexOf('(') + 1)
    if ($parts[0] -notmatch '(?s)^"((?:[^"\\]|\\.)*)"$') { throw "a message of $Id is formatted from something other than a literal: $e" }
    $format = ConvertFrom-RustString $Matches[1]
    $positional = @($parts | Select-Object -Skip 1)
    $text = [System.Text.StringBuilder]::new()
    $last = 0
    $next = 0
    foreach ($m in [regex]::Matches($format, '\{\{|\}\}|\{([^{}]*)\}')) {
        [void]$text.Append($format, $last, $m.Index - $last)
        $last = $m.Index + $m.Length
        if ($m.Value -eq '{{') { [void]$text.Append('{'); continue }
        if ($m.Value -eq '}}') { [void]$text.Append('}'); continue }
        $name = ($m.Groups[1].Value -split ':', 2)[0]
        $value = if ($name -eq '') { $positional[$next]; $next++ } elseif ($name -match '^\d+$') { $positional[[int]$name] } else { $name }
        [void]$text.Append('<' + (Get-Placeholder $value $Id) + '>')
    }
    [void]$text.Append($format, $last, $format.Length - $last)
    $text.ToString()
}

function Get-LineNumber([string] $Text, [int] $Index) { $Text.Substring(0, $Index).Split("`n").Count }

# The category each function that builds an error record gives it.
$errorSource = [System.IO.File]::ReadAllText((Join-Path $repo 'src/error.rs'))
$CategoryOf = @{}
foreach ($chunk in ($errorSource -split '\n(?=pub\(crate\) fn )')) {
    if ($chunk -match '(?s)^pub\(crate\) fn (\w+)\(id: &''static str.*?PsError::new\(ErrorCategory::(\w+), id,') {
        $CategoryOf[$Matches[1]] = $Matches[2]
    }
}
foreach ($helper in 'invalid', 'not_found', 'invalid_operation', 'device_error', 'unresolved') {
    if (-not $CategoryOf.ContainsKey($helper)) { throw "src/error.rs has no function $helper that builds an error record from an id" }
}

# What input_files in src/error.rs raises, with the category of each, as
# the id its caller gives it.
$inputFiles = [regex]::Match($errorSource, '(?s)pub\(crate\) fn input_files\(.*?\n\}')
if (-not $inputFiles.Success) { throw 'src/error.rs has no function input_files' }
$InputFileErrors = [System.Collections.Generic.List[object]]::new()
foreach ($call in [regex]::Matches($inputFiles.Value, '(?<![\w.])(?<helper>invalid|not_found|invalid_operation|unresolved)\(id, ')) {
    $helper = $call.Groups['helper'].Value
    $message = if ($helper -eq 'unresolved') { "Cannot find path '<path>' because it does not exist." } else {
        ConvertTo-MessageTemplate (Get-RustArgument $inputFiles.Value ($call.Index + $call.Length))[0] ''
    }
    if ($null -eq $message) { throw "input_files in src/error.rs raises a message held in a variable" }
    $InputFileErrors.Add([pscustomobject]@{ Category = $CategoryOf[$helper]; Message = $message })
}
if ($InputFileErrors.Count -eq 0) { throw 'input_files in src/error.rs raises nothing the reference can read' }
$deviceSource = [System.IO.File]::ReadAllText((Join-Path $repo 'src/device.rs'))
if ($deviceSource -notmatch '(?s)fn scoped<T>\(&self, id: &''static str, what: &str[^{]*\{.*?return Err\(device_error\(id, what, e\)\);') {
    throw 'Core::scoped in src/device.rs does not raise what wgpu reports as device_error(id, what, e)'
}
if ($deviceSource -notmatch '(?s)fn wait\(self, core: &Core[^{]*\{.*?Err\(device_error\(self\.id, self\.what, e\)\)') {
    throw 'Pending::wait in src/device.rs does not raise what wgpu reports of a submission as device_error(self.id, self.what, e)'
}

# Every call that raises an error with a literal id, or with a const that
# holds one, and every id literal anywhere else outside the tests.
$site = [regex]('(?<![\w.])(?:error::)?(?<helper>invalid|not_found|invalid_operation|device_error|unresolved)\(\s*"(?<id>Wgpu\w+)"' +
    '|(?<![\w.])(?<helper>input_files)\([^()"]*"(?<id>Wgpu\w+)"' +
    '|\.(?<helper>scoped|submit_as|begin_submit)\(\s*"(?<id>Wgpu\w+)"' +
    '|PsError::new\(\s*ErrorCategory::(?<category>\w+),\s*(?:"(?<id>Wgpu\w+)"|(?<const>[A-Z][A-Z0-9_]*)\b)')
$raised = @{}
foreach ($file in Get-ChildItem -LiteralPath (Join-Path $repo 'src') -Recurse -Filter '*.rs' | Sort-Object { [System.IO.Path]::GetRelativePath($repo, $_.FullName) }) {
    $text = [System.IO.File]::ReadAllText($file.FullName)
    $relative = [System.IO.Path]::GetRelativePath($repo, $file.FullName).Replace('\', '/')
    $tests = $text.IndexOf("`n#[cfg(test)]")
    if ($tests -lt 0) { $tests = $text.Length }
    $named = [System.Collections.Generic.HashSet[int]]::new()
    foreach ($m in $site.Matches($text)) {
        if ($m.Index -gt $tests) { continue }
        $where = "${relative}:$(Get-LineNumber $text $m.Index)"
        if ($m.Groups['const'].Success) {
            $const = $m.Groups['const'].Value
            $definition = [regex]::Match($text, "const $const\s*:\s*&(?:'static )?str\s*=\s*""(Wgpu\w+)""")
            if (-not $definition.Success) { throw "${where}: PsError::new takes $const, which is no const in this file holding an error id" }
            $id = $definition.Groups[1].Value
            [void]$named.Add($definition.Groups[1].Index - 1)
        } else {
            $id = $m.Groups['id'].Value
            [void]$named.Add($m.Groups['id'].Index - 1)
        }
        $parts = Get-RustArgument $text ($m.Index + $m.Length)
        $helper = $m.Groups['helper'].Value
        if ($helper -eq 'input_files') {
            if (-not $raised.ContainsKey($id)) {
                $raised[$id] = [pscustomobject]@{
                    Categories = [System.Collections.Generic.List[string]]::new()
                    Messages   = [System.Collections.Generic.List[string]]::new()
                }
            }
            foreach ($e in $InputFileErrors) {
                if (-not $raised[$id].Categories.Contains($e.Category)) { $raised[$id].Categories.Add($e.Category) }
                if (-not $raised[$id].Messages.Contains($e.Message)) { $raised[$id].Messages.Add($e.Message) }
            }
            continue
        }
        if ($helper -eq 'unresolved') {
            # PowerShell's own reason, which for a file that is not there is this.
            $category = $CategoryOf['unresolved']
            $messages = @("Cannot find path '<path>' because it does not exist.")
        } elseif ($helper -in 'scoped', 'submit_as', 'begin_submit', 'device_error') {
            $category = $CategoryOf['device_error']
            $what = ConvertTo-MessageTemplate $parts[0] $id
            if ($null -eq $what) { throw "${where}: the description $($parts[0]) of $id is held in a variable" }
            $reason = if ($helper -ne 'device_error') { '<reason>' } else {
                $rendered = ConvertTo-MessageTemplate $parts[1] $id
                if ($null -ne $rendered) { $rendered } else { "<$(Get-Placeholder $parts[1] $id)>" }
            }
            $messages = @("${what}: $reason")
        } else {
            $category = if ($helper) { $CategoryOf[$helper] } else { $m.Groups['category'].Value }
            $message = ConvertTo-MessageTemplate $parts[0] $id
            $messages = if ($null -ne $message) { @($message) } elseif ($MessagesOf.ContainsKey($id)) { $MessagesOf[$id] } else {
                throw "${where}: the message of $id is held in a variable; write it out in `$MessagesOf"
            }
        }
        if (-not $raised.ContainsKey($id)) {
            $raised[$id] = [pscustomobject]@{
                Categories = [System.Collections.Generic.List[string]]::new()
                Messages   = [System.Collections.Generic.List[string]]::new()
            }
        }
        if (-not $raised[$id].Categories.Contains($category)) { $raised[$id].Categories.Add($category) }
        foreach ($message in $messages) { if (-not $raised[$id].Messages.Contains($message)) { $raised[$id].Messages.Add($message) } }
    }
    foreach ($m in [regex]::Matches($text, '"Wgpu[A-Z]\w*"')) {
        if ($m.Index -gt $tests -or $named.Contains($m.Index)) { continue }
        if ($text.Substring(0, $m.Index) -match 'noun = $') { continue }
        throw "${relative}:$(Get-LineNumber $text $m.Index): $($m.Value) is neither the id of a call that raises an error nor a cmdlet's noun"
    }
}

# Every id the hand-written error lists name is one src/ raises.
foreach ($list in @($ErrorLists.Cmdlets.GetEnumerator()) + @($ErrorLists.Classes.GetEnumerator())) {
    foreach ($m in [regex]::Matches($list.Value, '`(Wgpu[A-Z]\w*)`')) {
        if (-not $raised.ContainsKey($m.Groups[1].Value)) { throw "tools/Error-Lists.psd1: the errors of $($list.Key) name $($m.Groups[1].Value), which nothing in src/ raises" }
    }
}

# A message as a code span in a table cell.
function Format-MessageCell([string] $Message) {
    $fence = if ($Message.Contains('`')) { '``' } else { '`' }
    $fence + ($Message -replace '\|', '\|') + $fence
}

$path = Join-Path $reference 'Error-Reference.md'
$tail = Get-HandWrittenTail $path
$page = Get-FrontMatter 'Error Reference' 110
$page.Add("Every one of the $($raised.Count) error ids the module and its Rust library raise, with its category and its messages. Source: the calls in ``src/`` that raise them, through ``invalid``, ``not_found``, ``invalid_operation``, ``device_error``, ``unresolved`` and ``input_files`` in ``src/error.rs``, through ``Core::scoped`` in ``src/device.rs``, which raises what wgpu reports while a call runs, through ``Core::submit_as`` and ``Core::begin_submit`` there, which raise what wgpu reports of a submission, or through ``PsError::new``, read by ``tools/Export-Reference.ps1``.")
$page.Add('')
$page.Add('A cmdlet writes an error as a non-terminating error record and goes on with its next input. The record''s `FullyQualifiedErrorId` is the id, a comma and the class of the cmdlet, as `WgpuNoAdapter,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand`, and its `CategoryInfo.Category` is the category below. A method throws instead: PowerShell wraps the error in a `MethodInvocationException`, whose `InnerException` is a `Pwrs.PwrsException` with a message that begins with the id in brackets, as `[WgpuRange] element 5 is past the end of a buffer of 3 elements`. The examples below read both.')
$page.Add('')
$page.Add('Text in angle brackets is filled in when the error is raised, and `<reason>` is what wgpu, the driver, naga or the operating system reported, in its own words. A path PowerShell cannot resolve is reported with PowerShell''s own reason, most often the one shown. A call that reaches the device checks it before and after, so most can also fail with `WgpuDeviceLost` once the device is lost, or with `WgpuUncaptured` when the device reported errors outside any call since it was last checked.')
$page.Add('')
$page.Add('## The ids')
$page.Add('')
$page.Add('| Id | Category | Message |')
$page.Add('|---|---|---|')
foreach ($id in ($raised.Keys | Sort-Object)) {
    $r = $raised[$id]
    $messages = @($r.Messages | ForEach-Object { Format-MessageCell $_ }) -join ', or '
    $page.Add("| ``$id`` | $(@($r.Categories | Sort-Object) -join ' or ') | $messages |")
}
$page.Add('')
foreach ($line in $tail) { $page.Add($line) }
Write-Page $path $page

# ---------------------------------------------------------------------
# The powershell/ pages: each group of cmdlets with its syntax and
# parameters, every class and enum on Types.md, and every cmdlet on the
# index, with no examples.
# ---------------------------------------------------------------------

$surfaceDir = Join-Path $reference 'powershell'
if (-not (Test-Path -LiteralPath $surfaceDir)) { New-Item -ItemType Directory -Path $surfaceDir | Out-Null }

$weight = 0
foreach ($pageName in $SurfacePages.Keys) {
    $weight += 10
    $spec = $SurfacePages[$pageName]
    $page = Get-FrontMatter $spec.Title $weight
    $named = Join-Word @($spec.Cmdlets | ForEach-Object { "``$_``" })
    $tasks = Join-Word @($spec.Tasks | ForEach-Object { "[$_]($_.md)" })
    $page.Add("The syntax, the parameters and the errors of $named. The examples are on $tasks.")
    $page.Add('')
    foreach ($cmdletName in $spec.Cmdlets) {
        $c = $cmdlets | Where-Object Name -eq $cmdletName
        $help = Get-Help $c.Name -Full
        $synopsis = $help.Synopsis.Trim()
        $page.Add("### $($c.Name)")
        $page.Add('')
        $page.Add((Format-Prose $synopsis))
        $page.Add('')
        $description = Get-HelpText $help.description
        if ($description -and $description -ne $synopsis) {
            foreach ($para in (Split-HelpText $description)) { $page.Add((Format-Prose $para)); $page.Add('') }
        }
        $page.Add("Alias: $(Format-CodeList @($aliasesOf[$c.Name]))")
        $page.Add('')
        $page.Add('```powershell')
        foreach ($line in ((Get-Command $c.Name -Syntax) -split "`r?`n" | Where-Object { $_.Trim() })) { $page.Add($line.Trim()) }
        $page.Add('```')
        $page.Add('')
        $names = @($c.Parameters.Keys | Where-Object { $_ -notin $common } | Sort-Object)
        if ($names.Count) {
            $page.Add('| Parameter | Type | Pipeline | Description |')
            $page.Add('|---|---|---|---|')
            foreach ($name in $names) {
                $meta = $c.Parameters[$name]
                $byValue = @($meta.ParameterSets.Values | Where-Object ValueFromPipeline).Count -gt 0
                $byName = @($meta.ParameterSets.Values | Where-Object ValueFromPipelineByPropertyName).Count -gt 0
                $pipeline = (@(if ($byValue) { 'by value' }) + @(if ($byName) { 'by name' })) -join ', '
                $paramHelp = @($help.parameters.parameter | Where-Object { $_.name -eq $name }) | Select-Object -First 1
                $text = Format-Cell (Get-HelpText $paramHelp.description)
                if ($meta.Aliases.Count) { $text = "$text Alias: $(Format-CodeList @($meta.Aliases | ForEach-Object { "-$_" }))." }
                $page.Add("| ``-$name`` | $(Format-SurfaceType $meta.ParameterType 'Types.md' -Parameter) | $pipeline | $($text.Trim()) |")
            }
            $page.Add('')
        }
        $outputs = @($c.OutputType)
        if ($outputs.Count) { $page.Add("Writes $(Join-Word @($outputs | ForEach-Object { Format-SurfaceOutput $_ }) 'or').") } else { $page.Add('Writes nothing to the pipeline.') }
        $page.Add('')
        $page.Add('#### Errors')
        $page.Add('')
        $page.Add($ErrorLists.Cmdlets[$c.Name])
        $page.Add('')
    }
    Write-Page (Join-Path $surfaceDir "$pageName.md") $page
}

# For each class, the methods of other classes that return it, by class,
# and the properties of other classes that hold it.
$returnedFrom = @{}
$heldIn = @{}
foreach ($t in ($classes.Values | Sort-Object FullName)) {
    foreach ($m in ($t.GetMethods([System.Reflection.BindingFlags]'Public,Instance,Static,DeclaredOnly') | Where-Object { -not $_.IsSpecialName } | Sort-Object MetadataToken)) {
        $inner = Get-InnerType $m.ReturnType
        if ($inner -and $classes.ContainsKey($inner.FullName) -and $inner.FullName -ne $t.FullName) {
            if (-not $returnedFrom.ContainsKey($inner.FullName)) { $returnedFrom[$inner.FullName] = [ordered]@{} }
            if (@($returnedFrom[$inner.FullName][$t.FullName]) -notcontains $m.Name) { $returnedFrom[$inner.FullName][$t.FullName] += @($m.Name) }
        }
    }
    foreach ($p in ($t.GetProperties([System.Reflection.BindingFlags]'Public,Instance,DeclaredOnly') | Sort-Object MetadataToken)) {
        $inner = Get-InnerType $p.PropertyType
        if ($inner -and $classes.ContainsKey($inner.FullName) -and $inner.FullName -ne $t.FullName) {
            $heldIn[$inner.FullName] += @("[$($t.FullName)](#$(Get-HeadingId $t.FullName)).$($p.Name)")
        }
    }
}

$page = Get-FrontMatter 'Types' 90
$page.Add("Every class and enumeration of the module, all in the ``Wgpu`` namespace: the $($classes.Count) classes the cmdlets write and take and the methods return, with the errors their methods throw, and the $($enums.Count) enumerations a parameter or a property takes, with each value's number. A property's type is the .NET type the module declares; where that is ``object``, the text beside it says what it holds. A parameter of an enumeration's type takes a value's name as a string, as ``-Backend Vulkan``, and PowerShell completes it.")
$page.Add('')
$page.Add('## Classes')
$page.Add('')
foreach ($t in ($classes.Values | Sort-Object FullName)) {
    $name = $t.FullName
    $page.Add("### $name")
    $page.Add('')
    foreach ($para in @(Get-DocParagraph $name)) { $page.Add((Format-Prose $para)); $page.Add('') }
    $origins = [System.Collections.Generic.List[string]]::new()
    if ($writers[$name]) {
        $links = @($writers[$name] | Sort-Object -Unique | ForEach-Object { '[{0}]({1}.md#{2})' -f $_, $surfaceOf[$_], (Get-HeadingId $_) })
        $origins.Add("Written by $(Join-Word $links).")
    }
    if ($returnedFrom[$name]) {
        $by = @(foreach ($owner in $returnedFrom[$name].Keys) {
                $methodNames = @($returnedFrom[$name][$owner])
                $noun = if ($methodNames.Count -gt 1) { 'methods' } else { 'method' }
                $codes = @($methodNames | ForEach-Object { '`{0}`' -f $_ })
                'the {0} {1} of [{2}](#{3})' -f (Join-Word $codes), $noun, $owner, (Get-HeadingId $owner)
            })
        $origins.Add("Returned by $(Join-Word $by).")
    }
    if ($heldIn[$name]) { $origins.Add("Held in $(Join-Word @($heldIn[$name] | Sort-Object -Unique)).") }
    if ($origins.Count) {
        $page.Add($origins -join ' ')
        $page.Add('')
    }
    $props = @($t.GetProperties([System.Reflection.BindingFlags]'Public,Instance,DeclaredOnly') | Sort-Object MetadataToken)
    if ($props.Count) {
        $page.Add('| Property | Type | Holds |')
        $page.Add('|---|---|---|')
        foreach ($p in $props) {
            $page.Add("| ``$($p.Name)`` | $(Format-SurfaceType $p.PropertyType '') | $(Format-Cell (@(Get-DocParagraph "$name.$($p.Name)") -join ' ')) |")
        }
        $page.Add('')
    }
    $ctors = @($t.GetConstructors() | Where-Object { $_.GetParameters().Count -gt 0 -or $Docs.ContainsKey("$name..ctor") })
    $methods = @($t.GetMethods([System.Reflection.BindingFlags]'Public,Instance,Static,DeclaredOnly') |
        Where-Object { -not $_.IsSpecialName -and $_.Name -notin 'ToString', 'Equals', 'GetHashCode' } | Sort-Object MetadataToken)
    if ($ctors.Count -or $methods.Count) {
        $page.Add('| Method | Returns | Does |')
        $page.Add('|---|---|---|')
        foreach ($k in $ctors) {
            $sig = ($k.GetParameters() | ForEach-Object { "$(Format-Type $_.ParameterType -Parameter) $($_.Name)" }) -join ', '
            $page.Add("| ``[$name]::new($sig)`` | [$name](#$(Get-HeadingId $name)) | $(Format-Cell (@(Get-DocParagraph "$name..ctor") -join ' ')) |")
        }
        foreach ($m in $methods) {
            $sig = ($m.GetParameters() | ForEach-Object { "$(Format-Type $_.ParameterType -Parameter) $($_.Name)" }) -join ', '
            $call = if ($m.IsStatic) { "[$name]::$($m.Name)($sig)" } else { "$($m.Name)($sig)" }
            $page.Add("| ``$call`` | $(Format-SurfaceType $m.ReturnType '') | $(Format-Cell (@(Get-DocParagraph "$name.$($m.Name)") -join ' ')) |")
        }
        $page.Add('')
    }
    if ($ErrorLists.Classes.ContainsKey($name)) {
        $page.Add('#### Errors')
        $page.Add('')
        $page.Add($ErrorLists.Classes[$name])
        $page.Add('')
    }
}
$page.Add('## Enumerations')
$page.Add('')
foreach ($e in $enums) {
    $name = $e.FullName
    $page.Add("### $name")
    $page.Add('')
    foreach ($para in @(Get-DocParagraph $name)) { $page.Add((Format-Prose $para)); $page.Add('') }
    $page.Add('| Value | Number | Means |')
    $page.Add('|---|---|---|')
    foreach ($value in [Enum]::GetNames($e)) {
        $number = [Convert]::ToInt64([Enum]::Parse($e, $value))
        $page.Add("| ``$value`` | $number | $(Format-Cell (@(Get-DocParagraph "$name.$value") -join ' ')) |")
    }
    $page.Add('')
}
Write-Page (Join-Path $surfaceDir 'Types.md') $page

$path = Join-Path $surfaceDir '_index.md'
$page = Get-HandWrittenHead $path '## Every cmdlet'
if ($null -eq $page) { $page = Get-FrontMatter 'Cmdlets and types' 100 }
$page.Add('## Every cmdlet')
$page.Add('')
$page.Add('| Cmdlet | Does |')
$page.Add('|---|---|')
foreach ($pageName in $SurfacePages.Keys) {
    foreach ($cmdletName in $SurfacePages[$pageName].Cmdlets) {
        $page.Add("| [$cmdletName]($pageName.md#$(Get-HeadingId $cmdletName)) | $(Format-Cell (Get-Help $cmdletName).Synopsis.Trim()) |")
    }
}
$page.Add('')
Write-Page $path $page

$undocumented = @()
foreach ($t in $classes.Values) {
    if (-not $Docs.ContainsKey($t.FullName)) { $undocumented += $t.FullName }
    foreach ($p in $t.GetProperties([System.Reflection.BindingFlags]'Public,Instance,DeclaredOnly')) {
        if (-not (Get-DocParagraph "$($t.FullName).$($p.Name)")) { $undocumented += "$($t.FullName).$($p.Name)" }
    }
    foreach ($m in $t.GetMethods([System.Reflection.BindingFlags]'Public,Instance,Static,DeclaredOnly') | Where-Object { -not $_.IsSpecialName -and $_.Name -notin 'ToString', 'Equals', 'GetHashCode' }) {
        if (-not (Get-DocParagraph "$($t.FullName).$($m.Name)")) { $undocumented += "$($t.FullName).$($m.Name)()" }
    }
}
foreach ($e in $enums) {
    if (-not $Docs.ContainsKey($e.FullName)) { $undocumented += $e.FullName }
    foreach ($value in [Enum]::GetNames($e)) { if (-not (Get-DocParagraph "$($e.FullName).$value")) { $undocumented += "$($e.FullName).$value" } }
}
"wrote the Error Reference with $($raised.Count) error ids, and under powershell/ $($SurfacePages.Count) cmdlet group pages for $($cmdlets.Count) cmdlets, the types page with $($classes.Count) classes, $($ErrorLists.Classes.Count) of them with their errors, and $($enums.Count) enums, and the index"
if ($undocumented.Count) { "no doc comment: $(($undocumented | Sort-Object) -join ', ')" }
