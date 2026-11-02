# Runs the Wgpu Pester tests whose full names match -FullName, from one test
# file, against a built module, with each test's outcome and every failure's
# message written out. The last line counts them; the exit code is the
# number that failed, or 99 when the filter selected no test. Patterns go
# through -Command, as -File hands '*a*','*b*' over as one string.
param(
    [Parameter(Mandatory)] [string] $Path,
    [Parameter(Mandatory)] [string[]] $FullName,
    [string] $ModuleDir = (Join-Path $PSScriptRoot '..\..\target\pwrs\Wgpu')
)
$ErrorActionPreference = 'Stop'
$env:PWRS_MODULE = $ModuleDir
Import-Module Pester -MinimumVersion 5.0
$config = New-PesterConfiguration
$config.Run.Path = $Path
$config.Run.PassThru = $true
$config.Filter.FullName = $FullName
$config.Output.Verbosity = 'Detailed'
$result = Invoke-Pester -Configuration $config
"pester filter host=$($PSVersionTable.PSVersion) pester=$((Get-Module Pester).Version) passed=$($result.PassedCount) failed=$($result.FailedCount) skipped=$($result.SkippedCount) notrun=$($result.NotRunCount)"
if ($result.PassedCount + $result.FailedCount + $result.SkippedCount -eq 0) {
    "pester filter selected no test for $($FullName -join ' | ')"
    exit 99
}
exit $result.FailedCount
