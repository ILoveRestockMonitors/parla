$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot 'install-rollback.ps1'
$tmp = Join-Path $env:TEMP ('parla-installer-test-' + [guid]::NewGuid().ToString('N'))
$release = Join-Path $tmp 'release'
$local = Join-Path $tmp 'local'
New-Item -ItemType Directory -Force -Path $release, $local | Out-Null
Set-Content -LiteralPath (Join-Path $release 'parla.exe') -Value 'fake-binary'
$hash = (Get-FileHash -LiteralPath (Join-Path $release 'parla.exe')).Hash
Set-Content -LiteralPath (Join-Path $release 'SHA256.txt') -Value "$hash  parla.exe"
$parlaDir = Join-Path $local 'Parla'
New-Item -ItemType Directory -Force -Path $parlaDir | Out-Null
$pbat = Join-Path $parlaDir 'parla.bat'
$tbat = Join-Path $parlaDir 'parla-turbo.bat'
Set-Content -LiteralPath $pbat -Value 'ORIGINAL-PARLA'
Set-Content -LiteralPath $tbat -Value 'ORIGINAL-TURBO'
$beforeP = (Get-FileHash -LiteralPath $pbat).Hash
$beforeT = (Get-FileHash -LiteralPath $tbat).Hash
$oldLocal = $env:LOCALAPPDATA
try {
  $env:LOCALAPPDATA = $local
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Prepare -Version test-1
  if ($LASTEXITCODE -ne 0) { throw 'Prepare failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version test-1
  if ($LASTEXITCODE -ne 0) { throw 'Activate failed' }
  $pa = Get-Content -LiteralPath $pbat -Raw
  $ta = Get-Content -LiteralPath $tbat -Raw
  $expectedLine='start "Parla engine" /min "'+(Join-Path $local 'Parla\releases\test-1\parla.exe')+'" %*'
  if((Get-Content -LiteralPath $pbat) -notcontains $expectedLine -or (Get-Content -LiteralPath $tbat) -notcontains $expectedLine){throw 'Launcher executable must be on one complete quoted line'}
  if ($pa -notmatch 'start "Parla engine" /min "' -or $pa -notmatch 'parla.exe" %\*') { throw 'quoted launcher assertion failed' }
  if ($ta -notmatch 'ggml-large-v3-turbo\.bin' -or $ta -match 'taskkill') { throw 'turbo launcher assertion failed' }
  Add-Content -LiteralPath (Join-Path $local 'Parla\releases\test-1\parla.exe') -Value 'tamper'
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version test-1
  if ($LASTEXITCODE -eq 0) { throw 'tampered staged executable was accepted' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Rollback -Version test-1
  if ($LASTEXITCODE -ne 0) { throw 'Rollback failed' }
  if ((Get-FileHash -LiteralPath $pbat).Hash -ne $beforeP -or (Get-FileHash -LiteralPath $tbat).Hash -ne $beforeT) { throw 'rollback byte comparison failed' }

  # A clean install has no launchers to preserve. Rollback must remove only
  # the two launchers created by activation.
  $cleanLocal=Join-Path $tmp 'clean-local'
  New-Item -ItemType Directory -Force -Path $cleanLocal | Out-Null
  $env:LOCALAPPDATA=$cleanLocal
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Prepare -Version clean-1
  if ($LASTEXITCODE -ne 0) { throw 'clean Prepare failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version clean-1
  if ($LASTEXITCODE -ne 0) { throw 'clean Activate failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version clean-1
  if ($LASTEXITCODE -ne 0) { throw 'repeat clean Activate failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -Action Rollback -Version clean-1
  if ($LASTEXITCODE -ne 0) { throw 'clean Rollback failed' }
  if ((Test-Path -LiteralPath (Join-Path $cleanLocal 'Parla\parla.bat')) -or (Test-Path -LiteralPath (Join-Path $cleanLocal 'Parla\parla-turbo.bat'))) { throw 'clean rollback left a launcher' }

  # A one-launcher install must preserve the existing file and absence of the
  # other launcher independently.
  $oneLocal=Join-Path $tmp 'one-local'
  New-Item -ItemType Directory -Force -Path $oneLocal | Out-Null
  $env:LOCALAPPDATA=$oneLocal
  $oneParla=Join-Path $oneLocal 'Parla\parla.bat'
  New-Item -ItemType Directory -Force -Path (Split-Path $oneParla -Parent) | Out-Null
  Set-Content -LiteralPath $oneParla -Value 'ONE-ORIGINAL'
  $oneHash=(Get-FileHash -LiteralPath $oneParla).Hash
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Prepare -Version one-1
  if ($LASTEXITCODE -ne 0) { throw 'one Prepare failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version one-1
  if ($LASTEXITCODE -ne 0) { throw 'one Activate failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -ReleaseDirectory $release -Action Activate -Version one-1
  if ($LASTEXITCODE -ne 0) { throw 'repeat one Activate failed' }
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scriptPath -Action Rollback -Version one-1
  if ($LASTEXITCODE -ne 0) { throw 'one Rollback failed' }
  if ((Get-FileHash -LiteralPath $oneParla).Hash -ne $oneHash -or (Test-Path -LiteralPath (Join-Path $oneLocal 'Parla\parla-turbo.bat'))) { throw 'one-launcher provenance rollback failed' }
  'installer-test-ok'
}
finally {
  $env:LOCALAPPDATA = $oldLocal
  $resolved = (Resolve-Path -LiteralPath $tmp).Path
  $tempRoot = (Resolve-Path -LiteralPath $env:TEMP).Path
  if ($resolved.StartsWith($tempRoot + '\') -and (Split-Path $resolved -Leaf) -like 'parla-installer-test-*') {
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}
