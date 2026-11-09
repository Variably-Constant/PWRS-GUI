//! The WebView2 add-on, `PWRS-GUI.WebView2.win-x64`: a module whose
//! `Install-GuiWebView2Runtime` puts Microsoft's WebView2 Fixed Version
//! runtime into the module's own `runtime` folder, where PWRS-GUI's HTML
//! renderer finds it. The command downloads the cab Microsoft publishes,
//! checks its SHA-256, expands it with Windows' `expand.exe`, and grants
//! the app containers WebView2's renderer runs in read and execute on the
//! folder, as Microsoft requires for a Fixed Version runtime shipped with an
//! unpackaged app on Windows 10. The package contains the manifest, the
//! command, a `license.txt` naming Microsoft's terms, which `Install-Module`
//! prompts the person to accept, and a `NOTICE` naming the runtime and those
//! terms; the runtime itself, 705.5 MB expanded, is not in it.

use crate::manifest::guid;
use std::path::Path;

/// The runtime's version, which is also the add-on's folder name and module
/// version.
const VERSION: &str = "155.0.4283.45";

/// The add-on's module name.
const NAME: &str = "PWRS-GUI.WebView2.win-x64";

/// Where Microsoft publishes the runtime's x64 cab, its SHA-256 and its
/// size in bytes.
const URL: &str = "https://msedge.sf.dl.delivery.mp.microsoft.com/filestreamingservice/files/01b0fbd9-6ea3-4e12-818a-a71f73f3855b/Microsoft.WebView2.FixedVersionRuntime.155.0.4283.45.x64.cab";
const SHA256: &str = "ec533bf1b7354d288703cf821a2d640d0130108224f93e330bb9c09faf3f8010";
const SIZE: u64 = 310_544_938;

/// The page Microsoft publishes the runtime and its license terms on.
const TERMS: &str = "https://developer.microsoft.com/microsoft-edge/webview2";

/// The third-party license files Microsoft ships in the runtime's folder.
const LICENSES: &[&str] = &[r"Trust Protection Lists\Mu\LICENSE", r"Trust Protection Lists\Sigma\LICENSE", r"WidevineCdm\LICENSE"];

/// Builds the add-on into `out`.
pub fn build(out: &Path) -> Result<(), String> {
    let addon = out.join(NAME).join(VERSION);
    if addon.exists() {
        return Err(format!("{} exists; give another output folder", addon.display()));
    }
    std::fs::create_dir_all(&addon).map_err(|e| format!("{} cannot be made: {e}", addon.display()))?;
    write(&addon.join(format!("{NAME}.psd1")), &manifest())?;
    write(&addon.join(format!("{NAME}.psm1")), &module())?;
    write(&addon.join("license.txt"), &license())?;
    write(&addon.join("NOTICE"), &notice())?;
    println!("{NAME} {VERSION}: the manifest, Install-GuiWebView2Runtime, license.txt and the NOTICE, in {}", addon.display());
    Ok(())
}

/// The add-on's module manifest: data alone, for Windows PowerShell 5.1 and
/// PowerShell 7. It requires the license to be accepted, so `Install-Module`,
/// `Save-Module` and `Update-Module` show `license.txt` and prompt for it,
/// unless given `-AcceptLicense`.
fn manifest() -> String {
    format!(
        "@{{\n    RootModule = '{NAME}.psm1'\n    ModuleVersion = '{VERSION}'\n    GUID = '{}'\n    Author = 'Mark Newton'\n    \
         Description = 'Microsoft''s WebView2 Fixed Version runtime {VERSION} for PWRS-GUI on Windows x64: Install-GuiWebView2Runtime puts it into this module''s folder, where PWRS-GUI''s HTML renderer starts on it.'\n    \
         CompatiblePSEditions = @('Desktop', 'Core')\n    PowerShellVersion = '5.1'\n    \
         FunctionsToExport = @('Install-GuiWebView2Runtime')\n    CmdletsToExport = @()\n    VariablesToExport = @()\n    AliasesToExport = @()\n    \
         PrivateData = @{{ PSData = @{{ Tags = @('PWRS-GUI', 'WebView2'); ProjectUri = 'https://github.com/Variably-Constant/PWRS-GUI'; \
         LicenseUri = '{TERMS}'; RequireLicenseAcceptance = $true }} }}\n}}\n",
        guid(NAME)
    )
}

/// The license `Install-Module` shows and prompts to accept: the runtime
/// the command downloads is Microsoft's, under Microsoft's terms, and the
/// module's own files are under PWRS-GUI's.
fn license() -> String {
    format!(
        "{NAME} {VERSION}\n\n\
         Install-GuiWebView2Runtime, which this module contains, downloads Microsoft's WebView2 Fixed Version runtime {VERSION} for x64 from Microsoft. \
         The runtime is Microsoft's software, under the Microsoft Software License Terms for the Microsoft Edge WebView2 Runtime, \
         which Microsoft shows where it publishes the runtime:\n  {TERMS}\n\n\
         Accepting this license agrees to those terms for the runtime this module downloads. \
         The module's own files are under PWRS-GUI's license, MIT.\n"
    )
}

/// The add-on's script module: `Install-GuiWebView2Runtime`, which does
/// nothing while the runtime is in place unless `-Force` is given.
fn module() -> String {
    format!(
        r#"# Install-GuiWebView2Runtime puts Microsoft's WebView2 Fixed Version
# runtime {VERSION} for x64 into this module's runtime folder: downloaded
# from Microsoft, checked against its SHA-256, expanded with expand.exe
# beside the folder, and granted to the app containers WebView2's renderer
# runs in. A runtime already there is swapped out by renames alone, once the
# new one is ready and while no process runs from it, so a failed install
# leaves it as it was.
function Install-GuiWebView2Runtime {{
    [CmdletBinding(SupportsShouldProcess)]
    param([switch] $Force)
    $ErrorActionPreference = 'Stop'
    $url = '{URL}'
    $sha256 = '{SHA256}'
    $runtime = Join-Path $PSScriptRoot 'runtime'
    if ((Test-Path -LiteralPath (Join-Path $runtime 'msedgewebview2.exe')) -and -not $Force) {{
        Write-Verbose "WebView2 {VERSION} is in $runtime already"
        return
    }}
    $root = [System.IO.Path]::GetPathRoot($PSScriptRoot)
    if ($root.StartsWith('\\') -or [System.IO.DriveInfo]::new($root).DriveType -eq 'Network') {{
        throw "this module lies in $PSScriptRoot, a network location, and a Fixed Version runtime does not run from one; install the module in a local folder"
    }}
    if (-not $PSCmdlet.ShouldProcess($runtime, 'Install Microsoft''s WebView2 Fixed Version runtime {VERSION}')) {{ return }}
    $old = Test-Path -LiteralPath $runtime
    if ($old) {{
        $users = @(Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($runtime + '\', [System.StringComparison]::OrdinalIgnoreCase) }})
        if ($users.Count) {{ throw "the runtime in $runtime is in use by the browser processes of the HTML windows drawing on it, $($users.Count) running; close those windows, then run this again" }}
    }}
    $system = Join-Path $env:SystemRoot 'System32'
    $staging = Join-Path $PSScriptRoot ('runtime-' + [guid]::NewGuid().ToString('n'))
    $replaced = Join-Path $staging 'replaced'
    $keep = $false
    New-Item -ItemType Directory -Path $staging | Out-Null
    try {{
        $cab = Join-Path $staging 'runtime.cab'
        if ($PSVersionTable.PSEdition -eq 'Desktop') {{
            [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        }}
        $ProgressPreference = 'SilentlyContinue'
        try {{
            Invoke-WebRequest -Uri $url -OutFile $cab -UseBasicParsing
        }} catch {{
            throw "the runtime could not be downloaded from ${{url}}: $_; if Microsoft no longer offers it, a newer PWRS-GUI.WebView2.win-x64 fetches a newer runtime"
        }}
        $got = (Get-FileHash -LiteralPath $cab -Algorithm SHA256).Hash
        if ($got -ne $sha256) {{ throw "the runtime downloaded from $url has SHA-256 $got, and this add-on takes $sha256" }}
        $expanded = Join-Path $staging 'expanded'
        New-Item -ItemType Directory -Path $expanded | Out-Null
        $said = & (Join-Path $system 'expand.exe') $cab -F:* $expanded
        if ($LASTEXITCODE -ne 0) {{ throw "expand.exe could not unpack the runtime, exit code ${{LASTEXITCODE}}: $said" }}
        $folder = Join-Path $expanded 'Microsoft.WebView2.FixedVersionRuntime.{VERSION}.x64'
        if (-not (Test-Path -LiteralPath (Join-Path $folder 'msedgewebview2.exe'))) {{ throw "the runtime's cab has no msedgewebview2.exe in $folder" }}
        foreach ($sid in '*S-1-15-2-1', '*S-1-15-2-2') {{
            $said = & (Join-Path $system 'icacls.exe') $folder /grant "${{sid}}:(OI)(CI)(RX)"
            if ($LASTEXITCODE -ne 0) {{ throw "the app containers WebView2's renderer runs in could not be given read access to ${{folder}}: $said" }}
        }}
        if ($old) {{
            try {{
                [System.IO.Directory]::Move($runtime, $replaced)
            }} catch {{
                throw "the runtime in $runtime could not be moved aside, so it is left as it was: $($_.Exception.GetBaseException().Message)"
            }}
        }}
        try {{
            [System.IO.Directory]::Move($folder, $runtime)
        }} catch {{
            $why = $_.Exception.GetBaseException().Message
            if ($old) {{
                try {{
                    [System.IO.Directory]::Move($replaced, $runtime)
                }} catch {{
                    $keep = $true
                    throw "the new runtime could not be moved into ${{runtime}}: $why; the runtime it replaces could not be moved back, and is in $replaced"
                }}
            }}
            throw "the new runtime could not be moved into ${{runtime}}, which is left as it was: $why"
        }}
    }} finally {{
        if (-not $keep -and (Test-Path -LiteralPath $staging)) {{
            try {{
                Remove-Item -LiteralPath $staging -Recurse -Force
            }} catch {{
                Write-Warning "$staging could not be removed: $_"
            }}
        }}
    }}
}}
"#
    )
}

/// What the add-on contains, what its command installs and from where, and
/// the terms the runtime is under.
fn notice() -> String {
    format!(
        "{NAME} {VERSION}\n\n\
         Install-GuiWebView2Runtime downloads Microsoft's WebView2 Fixed Version runtime {VERSION} for x64 from\n  {URL}\n\
         checks its SHA-256, {SHA256} ({SIZE} bytes), expands it with Windows' expand.exe into this module's runtime folder, \
         and grants the app containers WebView2's renderer runs in (S-1-15-2-1 and S-1-15-2-2) read and execute on that folder, \
         as Microsoft requires for a Fixed Version runtime shipped with an unpackaged app on Windows 10.\n\n\
         The runtime is Microsoft's, under the license terms Microsoft shows where it publishes the runtime:\n  {TERMS}\n\
         Its folder contains the license files Microsoft ships in it ({}), and show_third_party_software_licenses.bat, \
         which shows the credits of Chromium and the other open-source software the runtime is built from.\n\n\
         This module contains none of the runtime's files. A Fixed Version runtime does not run from a network location or a UNC path.\n",
        LICENSES.join(", ")
    )
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{} cannot be written: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_names_the_pinned_runtime_and_grants_both_app_containers() {
        let script = module();
        assert!(script.contains(URL) && script.contains(SHA256), "{script}");
        assert!(script.contains("*S-1-15-2-1") && script.contains("*S-1-15-2-2"), "{script}");
        assert!(script.contains(&format!("Microsoft.WebView2.FixedVersionRuntime.{VERSION}.x64")), "{script}");
        assert!(manifest().contains(&format!("ModuleVersion = '{VERSION}'")));
        assert!(notice().contains(TERMS) && notice().contains("WidevineCdm"));
    }

    #[test]
    fn installing_the_module_asks_for_microsofts_terms_to_be_accepted() {
        let manifest = manifest();
        assert!(manifest.contains("RequireLicenseAcceptance = $true") && manifest.contains(&format!("LicenseUri = '{TERMS}'")), "{manifest}");
        let license = license();
        assert!(license.contains("Microsoft Software License Terms for the Microsoft Edge WebView2 Runtime") && license.contains(TERMS), "{license}");
    }

    /// PowerShell 7's Move-Item copies a folder and deletes the original when
    /// a rename is denied, which leaves a runtime half deleted when one of its
    /// files is open; the runtime is swapped by renames alone.
    #[test]
    fn the_runtime_is_swapped_by_renames_alone() {
        let script = module();
        assert!(!script.contains("Move-Item"), "{script}");
        assert_eq!(script.matches("[System.IO.Directory]::Move(").count(), 3, "{script}");
    }
}
