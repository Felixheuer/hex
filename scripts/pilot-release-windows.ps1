# Builds and publishes Pilot's Windows release from the owner's Windows PC
# (docs/pilot-release.md): agent-desk.exe, its console entry point agent-desk.com, and the
# PilotSetup.exe installer, uploaded to ai.sms-pilot.de with scp; the
# server's pilot-publish.sh signs them with the release key and adds them to
# the release.
#
#   powershell -ExecutionPolicy Bypass -File scripts\pilot-release-windows.ps1 -Version 3.4.0-pilot.1
#     [-Channel stable] [-Arch x86_64|aarch64] [-Notes notes.txt] [-Server t3@ai.sms-pilot.de]
#     [-CertificateThumbprint <thumbprint>] [-NoUpload]
#
# Use the version the Linux build of this release has (the server's
# `pilot-publish.sh status` shows it), or a new one for a Windows-only release.
#
# Requirements: the build tools of docs/windows.md (Rust, Visual Studio C++
# tools, Vulkan SDK, Git for Windows with Git Bash), OpenSSH (ssh, scp), and
# an SSH key the server accepts. Inno Setup is installed on first use.
#
# Authenticode is optional. With -CertificateThumbprint (a code-signing
# certificate in Cert:\CurrentUser\My and signtool from the Windows SDK), the
# three files are signed and the build stamps the certificate's name
# (AGENT_DESK_AUTHENTICODE_SIGNER), so its updates must carry that signature too.
# Without it the files are unsigned (SmartScreen warns on first run), and
# Pilot's ed25519 release signature alone vouches for updates.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Version,
    [ValidateSet('stable', 'beta', 'dev')] [string]$Channel = 'stable',
    [ValidateSet('x86_64', 'aarch64')] [string]$Arch = 'x86_64',
    [string]$Notes,
    [string]$Server = 't3@ai.sms-pilot.de',
    [string]$Publish = '/srv/pilot/bin/pilot-publish.sh',
    [string]$CertificateThumbprint,
    [switch]$NoUpload
)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
if ($Version -notmatch '^\d+\.\d+\.\d+(-(beta|pilot)\.\d+)?$') { throw "Expected a version like 3.4.0-pilot.1, not $Version." }
if (git -C $root status --porcelain) { throw 'Commit or remove all working-tree changes before releasing.' }
$label = if ($Arch -eq 'x86_64') { 'x64' } else { 'arm64' }
$dist = Join-Path $root "dist\pilot-$Version"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null

$signer = $null
if ($CertificateThumbprint) {
    $certificate = Get-Item "Cert:\CurrentUser\My\$CertificateThumbprint"
    $signer = $certificate.GetNameInfo('SimpleName', $false)
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
    if (-not $signtool) { throw 'signtool.exe (Windows SDK) not found.' }
    Write-Output "Signing with $signer"
}
function Sign-File([string]$path) {
    if (-not $signer) { return }
    & $signtool sign /sha1 $CertificateThumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $path
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $path." }
}

$bash = (Get-Command bash.exe -ErrorAction SilentlyContinue | Where-Object { $_.Source -notlike '*\System32\*' } | Select-Object -First 1).Source
if (-not $bash) { $bash = "$env:ProgramFiles\Git\bin\bash.exe" }
if (-not (Test-Path $bash)) { throw 'Git Bash (bash.exe from Git for Windows) not found.' }

$env:AGENT_DESK_VERSION = $Version
$env:AGENT_DESK_BUILD_CHANNEL = $Channel
$env:AGENT_DESK_WINDOWS_ARCH = $Arch
$env:AGENT_DESK_RELEASE_DIST = $dist
if ($signer) { $env:AGENT_DESK_AUTHENTICODE_SIGNER = $signer } else { Remove-Item Env:AGENT_DESK_AUTHENTICODE_SIGNER -ErrorAction SilentlyContinue }
try {
    Push-Location $root
    & $bash scripts/build-windows.sh
    if ($LASTEXITCODE -ne 0) { throw "build-windows.sh failed ($LASTEXITCODE)." }
} finally {
    Pop-Location
    git -C $root checkout -- Cargo.toml Cargo.lock
}
$exe = Join-Path $dist "agent-desk-$Version-windows-$label.exe"
$com = Join-Path $dist "agent-desk-$Version-windows-$label.com"
Sign-File $exe
Sign-File $com
$setup = & (Join-Path $PSScriptRoot 'build-windows-installer.ps1') -Executable $exe -Console $com -Version $Version -Arch $Arch -OutDir $dist |
    Select-Object -Last 1
Sign-File $setup
Get-ChildItem $dist | Format-Table Name, Length

if (-not $NoUpload) {
    $incoming = "/srv/pilot/incoming/windows-$Version-$PID"
    ssh $Server "mkdir -p '$incoming'"
    if ($LASTEXITCODE -ne 0) { throw 'ssh failed.' }
    $files = @($exe, $com, $setup)
    if ($Notes) { $files += (Resolve-Path $Notes).Path }
    scp @files "${Server}:$incoming/"
    if ($LASTEXITCODE -ne 0) { throw 'scp failed.' }
    $remoteNotes = if ($Notes) { "--notes '$incoming/$(Split-Path -Leaf $Notes)'" } else { '' }
    $names = @($exe, $com, $setup) | ForEach-Object { "'$incoming/$(Split-Path -Leaf $_)'" }
    ssh $Server "'$Publish' publish --channel '$Channel' $remoteNotes '$Version' $($names -join ' ') && rm -rf '$incoming'"
    if ($LASTEXITCODE -ne 0) { throw 'Publishing on the server failed.' }
}
Write-Output "Pilot $Version for Windows: $setup"
Write-Output 'Install it with PilotSetup.exe (or the website); that install updates itself.'
