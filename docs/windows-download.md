# Install the Windows download

**[Download Parla (.exe)](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/parla.exe)** · **[Download Parla with setup helpers (.zip)](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-windows-x64.zip)**

Release: `0.2.0-terminal-insertion-20260916`, Windows x64. The download contains the tested native application, including the terminal insertion fix. You do not need Rust, GCC, Node.js, or a source checkout to use it. The EXE is the app, not an installer: a local speech engine and model must be configured before dictation works. The ZIP supplies the setup helpers used below.

## 1. Download and extract

Download `Parla-windows-x64.zip`, choose **Extract All**, and open Windows PowerShell in the extracted folder containing `parla.exe`. Keep that folder until installation is complete.

The [release page](https://github.com/ILoveRestockMonitors/parla/releases/latest) includes `SHA256SUMS.txt` for the download assets. To check the extracted executable against its included checksum:

```powershell
$Expected = ((Get-Content -LiteralPath .\SHA256.txt -TotalCount 1) -split '\s+')[0]
$Actual = (Get-FileHash -LiteralPath .\parla.exe -Algorithm SHA256).Hash
if ($Actual -ne $Expected) { throw 'Checksum mismatch; download a fresh copy.' }
```

The executable is unsigned; Windows may show an unknown-publisher or SmartScreen prompt. Obtain it from the linked repository release. Do not disable Windows security protections. Checksums detect changed downloads but are not a publisher signature.

## 2. Set up local speech recognition

If you already use Parla with working settings, keep those settings and skip to step 3. For a new installation:

1. Download a Windows x64 distribution from [whisper.cpp releases](https://github.com/ggml-org/whisper.cpp/releases) that includes **`whisper-server.exe`**, and extract it with all of its matching DLLs. CPU builds avoid requiring CUDA. A package containing only `whisper-cli.exe` cannot serve Parla.
2. Download **`ggml-small.bin`** from the [Whisper ggml model repository](https://huggingface.co/ggerganov/whisper.cpp). Keep the server and model in a durable location, such as `%LOCALAPPDATA%\Parla\engines\whisper` and `%LOCALAPPDATA%\Parla\models\whisper`.
3. Run the settings helper from the extracted Parla download. Replace the example paths with the actual files:

```powershell
$WhisperExe = 'C:\path\to\whisper-server.exe'
$WhisperModel = 'C:\path\to\ggml-small.bin'
& $WhisperExe --help
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\initialize-settings.ps1 -WhisperServerExe $WhisperExe -WhisperModelPath $WhisperModel
if ($LASTEXITCODE -ne 0) { throw 'Settings initialization failed.' }
```

Keep the server's own license files with its distribution. If `--help` reports missing DLLs, repair that distribution before continuing. The settings helper refuses to overwrite existing settings. It enables Faithful mode and 24-hour auto-delete history; add `-HistoryMode never` to disable new transcript-history writes. No Ollama model is required for Faithful mode. Python is needed only if you choose the optional Parakeet recognition backend.

For optional Parakeet or Ollama, use the [complete guide](https://github.com/ILoveRestockMonitors/parla/blob/main/PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md#10-optional-parakeet-backend). Skip its source extraction/build steps and retain the release version below.

## 3. Install the app and launchers

Run from the extracted Parla folder in a normal, non-administrator PowerShell window:

```powershell
$Release = (Get-Location).Path
$BuildId = '0.2.0-terminal-insertion-20260916'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-rollback.ps1 -ReleaseDirectory $Release -Action Prepare -Version $BuildId
if ($LASTEXITCODE -ne 0) { throw 'Release staging failed.' }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-rollback.ps1 -ReleaseDirectory $Release -Action Activate -Version $BuildId
if ($LASTEXITCODE -ne 0) { throw 'Launcher activation failed.' }
& (Join-Path $env:LOCALAPPDATA 'Parla\parla.bat')
```

This installs the executable under `%LOCALAPPDATA%\Parla\releases\<version>` and creates `parla.bat` and `parla-turbo.bat` in `%LOCALAPPDATA%\Parla`. The normal launcher starts Parla and opens its dashboard at **http://127.0.0.1:9393/**. You can make a desktop shortcut to `parla.bat` for subsequent launches. Turbo needs its own separately downloaded large-v3-turbo model; use the normal launcher initially.

An already-running Parla instance remains running. When upgrading, finish recording/processing, close only the existing Parla process in Task Manager, and then launch the newly staged version. Settings and dictionaries remain in `%LOCALAPPDATA%\Parla`; the download contains no personal settings, recordings, history, or dictionary database. Previous launchers are backed up for rollback.

## 4. Try dictation

In the dashboard, check that the microphone, keyboard shortcut, and speech engine are ready. Windows must permit microphone access for desktop applications. In a disposable Notepad document, press **Ctrl+Space**, speak, and press it again to finish; or hold **Ctrl+Win** while speaking. The first speech-engine load may take longer. If recognition is unavailable, check the dashboard's error and configured server/model paths.

Keep the dashboard and model services on localhost. Ordinary application compatibility and recognition quality depend on the computer, model, and target editor; the published checks do not cover every environment.

## Roll back launchers

From the extracted ZIP folder:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-rollback.ps1 -Action Rollback -Version '0.2.0-terminal-insertion-20260916'
if ($LASTEXITCODE -ne 0) { throw 'Launcher rollback failed.' }
```

Rollback restores the previous launchers, or removes launchers that did not exist before activation. It does not stop processes or delete settings, models, dictionaries, or staged executables. Close the current Parla process between dictations before launching a previous version.

## Release identity

- Binary source commit: [`e0634eed28c13c9ad30a333742814f5790b440ae`](https://github.com/ILoveRestockMonitors/parla/commit/e0634eed28c13c9ad30a333742814f5790b440ae).
- EXE SHA-256: `1E533933A359E07B2EE63ADB9C95F48C8B9A5454744402DC48ADDE9F774BF443`.
- Original release verification: 122 Rust tests passed, 4 opt-in tests ignored; 3 CLI checks and the read-only terminal-provider check passed; the owner confirmed 5/5 successful Hermes dictations.
- Download verification: original executable hash and Windows x64 format checked; the isolated CLI checks rerun against the packaged executable. No new live microphone or application-insertion test is implied.

The ZIP includes `RELEASE.json` and `THIRD-PARTY-NOTICES.txt`. See the [current state](https://github.com/ILoveRestockMonitors/parla/blob/main/docs/current-state.md) for known application limitations.
