<#
Runs an ExtendScript file in Photoshop through COM and prints its result,
handing it -Arguments as its `arguments`. Starts Photoshop when it is not
running and leaves it running afterwards; close it when the batches are
done. Fails when the script's result begins with ERROR.

    pwsh -NoProfile -File ps_run.ps1 -Script <file.jsx> [-Arguments <value>...]
#>
param(
    [Parameter(Mandatory)] [string] $Script,
    [string[]] $Arguments = @()
)
$ErrorActionPreference = 'Stop'
$path = (Resolve-Path -LiteralPath $Script).Path
"start $(Get-Date -Format o) $path"
$before = @(Get-Process | Where-Object ProcessName -eq 'Photoshop')
"Photoshop processes before: $($before.Count)"
$app = New-Object -ComObject Photoshop.Application
"connected to $($app.Name) $($app.Version)"
# ExecutionMode 1: never show the debugger.
$result = $app.DoJavaScriptFile($path, [object[]]$Arguments, 1)
"result: $result"
"end $(Get-Date -Format o)"
if ("$result".StartsWith('ERROR')) {
    throw "the script failed: $result"
}
