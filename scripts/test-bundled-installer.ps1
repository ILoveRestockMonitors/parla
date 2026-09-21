param([Parameter(Mandatory=$true)][string]$Installer,
      [Parameter(Mandatory=$true)][string]$Python,
      [Parameter(Mandatory=$true)][string]$Fixture)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$key='HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/{61AF8F6E-9BC6-445F-90D4-8F2D334071E9}_is1'
if(Test-Path -LiteralPath $key){throw 'An installation is already registered; use another test account'}
$installerPath=(Resolve-Path -LiteralPath $Installer).Path
$testRoot=Join-Path $env:TEMP ('parla-bundled-install-test-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $testRoot 'Parla App'
$data=Join-Path $testRoot 'User Data'
New-Item -ItemType Directory -Path $data -Force | Out-Null
$oldLocal=$env:LOCALAPPDATA
$oldData=$env:PARLA_DATA_DIR
$record=[ordered]@{test_root=$testRoot;installer=$installerPath}
try {
    $env:LOCALAPPDATA=$data
    $env:PARLA_DATA_DIR=Join-Path $data 'Parla'
    $arguments=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/NOICONS','/TASKS=""',('/DIR="'+$app+'"'),('/LOG="'+(Join-Path $testRoot 'install.log')+'"'))
    $process=Start-Process -FilePath $installerPath -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    if($process.ExitCode -ne 0){throw "Install failed: $($process.ExitCode)"}
    $settings=Join-Path $data 'Parla/settings.json'
    if(!(Test-Path -LiteralPath $settings)){throw 'Fresh settings did not land in isolated user data'}
    $value=Get-Content -LiteralPath $settings -Raw | ConvertFrom-Json
    if($value.asr_backend -ne 'parakeet' -or $value.cleanup_mode -ne 'faithful' -or $value.stutter_correction -ne $true){throw 'Incorrect first-run defaults'}
    if([IO.Path]::GetFullPath($value.asr_server_exe) -ne [IO.Path]::GetFullPath((Join-Path $app 'engines/whisper/whisper-server.exe'))){throw 'Installed paths were not initialized'}
    $manifest=Get-Content -LiteralPath (Join-Path $app 'bundle-manifest.json') -Raw | ConvertFrom-Json
    foreach($file in $manifest.files){
        $path=[IO.Path]::GetFullPath((Join-Path $app $file.path))
        if(!$path.StartsWith($app+'\')){throw 'Manifest path outside app directory'}
        if((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256){throw "Installed hash mismatch: $($file.path)"}
    }
    $record.verified_files=$manifest.files.Count
    & $Python (Join-Path $root 'tests/integration/bundled_runtime_test.py') $app $Fixture *> (Join-Path $testRoot 'runtime.json')
    if($LASTEXITCODE -ne 0){Get-Content -LiteralPath (Join-Path $testRoot 'runtime.json'); throw 'Installed runtime checks failed'}
    $record.runtime=Get-Content -LiteralPath (Join-Path $testRoot 'runtime.json') -Raw | ConvertFrom-Json
    $settingsHash=(Get-FileHash -LiteralPath $settings).Hash
    $uninstaller=(Resolve-Path -LiteralPath (Join-Path $app 'unins000.exe')).Path
    if(!$uninstaller.StartsWith($testRoot+'\')){throw 'Uninstaller is outside this test installation'}
    $process=Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -PassThru -Wait
    if($process.ExitCode -ne 0){throw "Uninstall failed: $($process.ExitCode)"}
    if(Test-Path -LiteralPath (Join-Path $app 'parla.exe')){throw 'Uninstall left the app executable'}
    if(Test-Path -LiteralPath $key){throw 'Uninstall left registration'}
    if((Get-FileHash -LiteralPath $settings).Hash -ne $settingsHash){throw 'Uninstall changed personal settings'}
    $record.settings_preserved_after_uninstall=$true
    $record.install_and_uninstall='passed'
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $root 'release/bundled-installer-test.json') -Encoding utf8
    $record | ConvertTo-Json -Depth 8
} finally { $env:LOCALAPPDATA=$oldLocal; $env:PARLA_DATA_DIR=$oldData }
