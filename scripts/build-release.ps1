param(
    [Parameter(Mandatory=$true)][string]$CompilerBin,
    [string]$Toolchain='stable-x86_64-pc-windows-gnu',
    [string]$OutputDirectory=''
)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$compiler=(Resolve-Path -LiteralPath $CompilerBin).Path
foreach($name in @('gcc.exe','ar.exe')){
    if(!(Test-Path -LiteralPath (Join-Path $compiler $name) -PathType Leaf)){throw "Missing $name in CompilerBin"}
}
$cargo=(Get-Command cargo -ErrorAction SilentlyContinue).Source
if(!$cargo){$cargo=Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'}
if(!(Test-Path -LiteralPath $cargo -PathType Leaf)){throw 'Install Rust/rustup first'}
if($Toolchain -notmatch '^[A-Za-z0-9._-]+-x86_64-pc-windows-gnu$'){throw 'This recipe requires a Windows x64 GNU toolchain'}
$version=(Get-Content -LiteralPath (Join-Path $root 'VERSION') -Raw).Trim()
$target=Join-Path $env:TEMP 'parla-portable-build'
$env:CARGO_TARGET_DIR=$target
$env:CC=Join-Path $compiler 'gcc.exe'
$env:AR=Join-Path $compiler 'ar.exe'
$env:PATH=$compiler+';'+$env:PATH
$env:PARLA_BUILD_REVISION=(& git -C $root rev-parse HEAD)
& $cargo "+$Toolchain" test --locked --offline --manifest-path (Join-Path $root 'Cargo.toml')
if($LASTEXITCODE -ne 0){throw 'Tests failed; do not package this build'}
& $cargo "+$Toolchain" build --release --locked --offline --manifest-path (Join-Path $root 'Cargo.toml')
if($LASTEXITCODE -ne 0){throw 'Release build failed'}
if(!$OutputDirectory){$OutputDirectory=Join-Path $root 'release'}
$out=Join-Path $OutputDirectory $version
New-Item -ItemType Directory -Force -Path $out | Out-Null
$exe=Join-Path $out 'parla.exe'
Copy-Item -LiteralPath (Join-Path $target 'release\parla.exe') -Destination $exe -Force
$hash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
Set-Content -LiteralPath (Join-Path $out 'SHA256.txt') -Value "$hash  parla.exe"
[ordered]@{version=$version;source_commit=$env:PARLA_BUILD_REVISION;sha256=$hash.ToLowerInvariant();platform='windows-x64'} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $out 'build-record.json') -Encoding utf8
Write-Output "Release verified and packaged: $out"
