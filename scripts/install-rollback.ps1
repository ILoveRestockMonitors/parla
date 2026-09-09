param(
    [string]$ReleaseDirectory='',
    [ValidateSet('Prepare','Activate','Rollback')][string]$Action='Prepare',
    [string]$Version='0.2.0-insertion-20260908'
)
$ErrorActionPreference='Stop'
if($Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'){throw 'Invalid version'}
if(!$env:LOCALAPPDATA){throw 'LOCALAPPDATA unavailable'}
$base=[IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Parla'))
$dest=Join-Path $base ('releases\'+$Version)
$backup=Join-Path $base ('rollback\'+$Version)
$names=@('parla.bat','parla-turbo.bat')

# Recovery must remain available even if the new binary was damaged or lost.
if($Action -eq 'Rollback'){
    foreach($name in $names){
        $old=Join-Path $backup $name
        $missing="$old.missing"
        $current=Join-Path $base $name
        if((Test-Path -LiteralPath $old) -and (Test-Path -LiteralPath $missing)){throw "Rollback manifest is ambiguous: $name"}
        if(!(Test-Path -LiteralPath $old) -and !(Test-Path -LiteralPath $missing)){throw "Rollback launcher provenance missing: $old"}
    }
    foreach($name in $names){
        $old=Join-Path $backup $name
        $missing="$old.missing"
        $current=Join-Path $base $name
        if(Test-Path -LiteralPath $old){Copy-Item -LiteralPath $old -Destination $current -Force}
        elseif(Test-Path -LiteralPath $current){Remove-Item -LiteralPath $current -Force}
    }
    return
}
if(!$ReleaseDirectory){throw 'ReleaseDirectory is required for Prepare and Activate'}
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
$exe=Join-Path $release 'parla.exe'
$manifest=Join-Path $release 'SHA256.txt'
if(!(Test-Path -LiteralPath $exe)-or !(Test-Path -LiteralPath $manifest)){throw 'Missing release files'}
$expected=((Get-Content -LiteralPath $manifest -TotalCount 1)-split '\s+')[0].ToUpperInvariant()
$actual=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToUpperInvariant()
if($expected -notmatch '^[A-F0-9]{64}$' -or $expected -ne $actual){throw 'SHA256 verification failed'}
$installed=Join-Path $dest 'parla.exe'
if($Action -eq 'Prepare'){
    if((Test-Path -LiteralPath $installed) -and (Get-FileHash -LiteralPath $installed).Hash -ne $actual){throw 'A different binary already uses this version; choose a new version'}
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    Copy-Item -LiteralPath $exe -Destination $installed -Force
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $dest 'SHA256.txt') -Force
    return
}
if(!(Test-Path -LiteralPath $installed) -or (Get-FileHash -LiteralPath $installed).Hash -ne $actual){throw 'Staged executable missing or hash mismatch'}
New-Item -ItemType Directory -Force -Path $backup | Out-Null
foreach($name in $names){
    $src=Join-Path $base $name
    $old=Join-Path $backup $name
    $missing="$old.missing"
    $hasOld=Test-Path -LiteralPath $old
    $hasMissing=Test-Path -LiteralPath $missing
    if($hasOld -and $hasMissing){throw "Rollback manifest is ambiguous: $name"}
    if(!$hasOld -and !$hasMissing){
        if(Test-Path -LiteralPath $src){Copy-Item -LiteralPath $src -Destination $old}
        else {Set-Content -LiteralPath $missing -Value 'absent' -NoNewline}
    }
}
$launch='start "Parla engine" /min "'+$installed+'" %*'
$openDashboard='start "" "http://127.0.0.1:9393"'
$pythonEnv='if exist "%LOCALAPPDATA%\Parla\venv\Scripts\python.exe" set "PYTHON=%LOCALAPPDATA%\Parla\venv\Scripts\python.exe"'
$normal=@('@echo off',$pythonEnv,$launch,$openDashboard)
$turbo=@('@echo off',$pythonEnv,'set "PARLA_MODEL=%LOCALAPPDATA%\Parla\models\whisper\ggml-large-v3-turbo.bin"',$launch,$openDashboard)
Set-Content -LiteralPath (Join-Path $base 'parla.bat') -Value $normal
Set-Content -LiteralPath (Join-Path $base 'parla-turbo.bat') -Value $turbo
