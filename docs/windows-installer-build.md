# Building the Windows offline installer

The installer is a Windows x64 per-user package. End users need only `Parla-Setup.exe`; the tools below are build prerequisites for maintainers.

Use the Rust GNU toolchain and a complete WinLibs distribution containing GCC, binutils, CMake, and Ninja. Use Python 3.12 or newer for packaging scripts and Inno Setup 6.6.1 for the wizard. Run commands from a fresh source checkout outside folders being rewritten by another sync process.

## Assemble and build

```powershell
$compiler = 'C:\path\to\mingw64\bin'
$python = 'C:\path\to\python.exe'
$iscc = 'C:\path\to\Inno Setup 6\ISCC.exe'

cargo +stable-x86_64-pc-windows-gnu fetch --locked --manifest-path .\Cargo.toml
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-bundled-whisper.ps1 -CompilerBin $compiler
& $python .\scripts\prepare-bundle.py
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-release.ps1 -CompilerBin $compiler
& $python .\scripts\finalize-bundle.py

$payload = (Resolve-Path .\release\bundle-payload).Path
$output = Join-Path (Get-Location).Path 'release\0.2.0-bundled-20260919\installer'
& $iscc ('/DPayloadDir=' + $payload) ('/DOutputDir=' + $output) .\scripts\parla-installer.iss
```

Whisper is built from verified revision `48f628a84833905ee4a0658ee6d4a5c915ce1997` (v1.8.7) with static compiler runtimes, OpenMP disabled, and optional AVX/FMA instruction paths disabled. Its imports are checked for external compiler-runtime dependencies. This avoids a separate Visual C++ runtime installation.

`bundle-sources.json` pins the speech models, Python distribution, wheels, and installer build tool by SHA-256. Preparation downloads and verifies these files, creates the private interpreter and model directories, and refuses to overwrite an existing payload. The Inno download is a build tool; it is not installed on end users' PCs. Preserve prior payloads before a fresh assembly.

Finalization copies the tested application, installs attribution and license notices, and creates `bundle-manifest.json` with hashes for installed files. Run it after changing the app or welcome text and before compiling the installer. The current Rust notice bundle is pinned to the earlier release with the same Cargo.lock; regenerate notices if dependencies change. Python bytecode caches are excluded from the installer.

## Verify before publishing

```powershell
& $python .\tests\integration\bundled_runtime_test.py .\release\bundle-payload .\release\bundle-fixtures\0.wav
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-bundled-installer.ps1 -Installer .\release\0.2.0-bundled-20260919\installer\Parla-Setup.exe -Python $python -Fixture .\release\bundle-fixtures\0.wav
Get-FileHash .\release\0.2.0-bundled-20260919\installer\Parla-Setup.exe -Algorithm SHA256
```

The installer test refuses an already registered Parla installation. Use a separate test account when necessary. It installs silently into a unique temporary directory, with isolated user data and no shortcuts or automatic launch. It checks first-run settings, every manifest hash, both recognizers, uninstallation, and retained personal settings. On failure, inspect the printed test directory; use only that test installation's uninstaller before retrying. These checks do not replace clean Windows VM and manual microphone/shortcut testing.

Publish the installer, `SHA256SUMS.txt`, `bundle-manifest.json`, `THIRD-PARTY-NOTICES.txt`, and a release record identifying the source commit and verification scope. Check uploaded asset digests and the public download before marking the release ready. The current app and installer are unsigned.
