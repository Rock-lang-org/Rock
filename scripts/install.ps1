# Bootstrap the manager on Windows, then use it to install the selected toolchain.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1 [vVERSION]
#   & ([scriptblock]::Create((irm https://github.com/Rock-lang-org/Rock/releases/latest/download/install.ps1))) [vVERSION]
param([string]$Channel = 'stable', [Parameter(ValueFromRemainingArguments)] $Extra)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    [Console]::Error.WriteLine("rock installer: $Message")
    exit 1
}

if ($Extra) { Fail 'Usage: install.ps1 [vVERSION]' }
if ($Channel -eq 'stable') {
    $releasePath = 'latest/download'
} elseif ($Channel.Length -le 128 -and $Channel -cmatch '^v[0-9]+(\.[0-9]+)*([-+][A-Za-z0-9._+-]+)?$') {
    $releasePath = "download/$Channel"
} else {
    Fail 'Expected a version such as v0.1.0 (default: stable)'
}

# A 32-bit PowerShell on 64-bit Windows reports the real CPU here.
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') { Fail "Only x86_64-pc-windows-msvc is supported, not $arch" }

if ($env:ROCKUP_HOME) {
    if (-not [IO.Path]::IsPathRooted($env:ROCKUP_HOME)) { Fail 'ROCKUP_HOME must be an absolute path' }
    $rockupHome = $env:ROCKUP_HOME
} else {
    if (-not $env:USERPROFILE) { Fail 'USERPROFILE must be set' }
    $rockupHome = Join-Path $env:USERPROFILE '.rockup'
}
$installed = Join-Path $rockupHome 'bin\rockup.exe'
if (Test-Path -LiteralPath $installed) {
    Fail "Refusing to overwrite $installed; use the existing rockup install $Channel, rockup update, or rockup self update"
}

# curl.exe ships with Windows 10 1803 and later; `curl` alone may be the
# Invoke-WebRequest alias. The flags match install.sh and rockup.
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) { Fail 'Required command not found: curl.exe' }
$stage = Join-Path ([IO.Path]::GetTempPath()) ("rock-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    $asset = 'rockup-x86_64-pc-windows-msvc.exe'
    $base = "https://github.com/Rock-lang-org/Rock/releases/$releasePath"
    foreach ($file in @($asset, "$asset.sha256")) {
        & curl.exe --disable --fail --silent --show-error --location `
            --proto '=https' --proto-redir '=https' `
            --retry 3 --connect-timeout 30 --max-time 1800 `
            --output (Join-Path $stage $file) "$base/$file"
        if ($LASTEXITCODE -ne 0) {
            Fail "Download failed: $base/$file (a release with bootstrap assets must be published first)"
        }
    }

    # Validate the sidecar: one sha256sum line naming the exact asset.
    $lines = @((Get-Content -LiteralPath (Join-Path $stage "$asset.sha256") -Raw).TrimEnd("`r", "`n") -split "`r?`n")
    if ($lines.Count -ne 1 -or $lines[0] -cnotmatch '^([0-9a-fA-F]{64}) [ *](.+)$' -or $Matches[2] -cne $asset) {
        Fail 'Invalid SHA-256 sidecar: expected one line naming the exact rockup asset'
    }
    $expected = $Matches[1]
    $downloaded = Join-Path $stage $asset
    if ((Get-FileHash -LiteralPath $downloaded -Algorithm SHA256).Hash -ne $expected.ToUpperInvariant()) {
        Fail 'SHA-256 verification failed'
    }
    if (Test-Path -LiteralPath $installed) {
        Fail "rockup appeared at $installed during download; refusing to overwrite it"
    }
    & $downloaded self install
    if ($LASTEXITCODE -ne 0) { Fail 'rockup manager installation failed' }
    if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) { Fail "rockup did not persist itself at $installed" }
    & $installed install $Channel
    if ($LASTEXITCODE -ne 0) { Fail "toolchain installation failed; retry with: & '$installed' install $Channel" }
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host ''
Write-Host 'Rock installed. Open a new terminal, then try:'
Write-Host '  rock --version'
Write-Host 'Compiling Rock programs needs the MSVC linker (Visual Studio Build Tools).'
