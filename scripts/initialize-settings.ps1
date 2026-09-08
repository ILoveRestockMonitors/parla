param(
    [Parameter(Mandatory=$true)][string]$WhisperServerExe,
    [Parameter(Mandatory=$true)][string]$WhisperModelPath,
    [string]$FormatterModel='qwen2.5:3b',
    [ValidateSet('auto_delete_24h','store','never')][string]$HistoryMode='auto_delete_24h'
)
$ErrorActionPreference='Stop'
if(!$env:LOCALAPPDATA){throw 'LOCALAPPDATA is unavailable'}
foreach($path in @($WhisperServerExe,$WhisperModelPath)){
    if(!(Test-Path -LiteralPath $path -PathType Leaf)){throw "Required local file does not exist: $path"}
}
$base=Join-Path $env:LOCALAPPDATA 'Parla'
$target=Join-Path $base 'settings.json'
if(Test-Path -LiteralPath $target){throw 'settings.json already exists. Back it up and edit it deliberately; this first-run script never overwrites it.'}
$settings=[ordered]@{
    schema_version=2; ptt_chords=@('ctrl+win'); toggle_chord='ctrl+space'
    asr_backend='whisper'
    asr_server_exe=(Resolve-Path -LiteralPath $WhisperServerExe).Path
    asr_model_path=(Resolve-Path -LiteralPath $WhisperModelPath).Path
    parakeet_model_dir=(Join-Path $base 'models\parakeet')
    formatter_port=11434; formatter_model=$FormatterModel; formatter_num_ctx=2048
    cleanup_mode='faithful'; history_mode=$HistoryMode
    chimes_enabled=$true; hud_enabled=$true
    max_recording_seconds=1200; max_pending_utterances=2
    retain_audio_for_retry=$false; retry_audio_seconds=120
    microphone_name=$null; min_speech_rms=0.001; capture_drain_ms=80
}
New-Item -ItemType Directory -Path $base -Force | Out-Null
$bytes=[Text.Encoding]::UTF8.GetBytes(($settings | ConvertTo-Json -Depth 4))
$stream=[IO.File]::Open($target,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try {$stream.Write($bytes,0,$bytes.Length); $stream.Flush($true)} finally {$stream.Dispose()}
Write-Output "Created first-run settings: $target"
Write-Output 'Faithful mode needs no formatter model. Parla has not been started.'
