# Parla on macOS and Linux

The portable packaging target is `0.3.0-portable-20260921`. Download only artifacts
attached to a published Parla release and compare their SHA-256 checksum with that
release's checksum file. A workflow definition or successful compile is not an
installer release. The release's verification files identify which native builds
and installed-package checks actually passed.

## Included speech engine

These packages contain the native Parla executable, Python 3.12.14 from Astral's
Python Build Standalone release 20260901, sherpa-onnx/core 1.13.7, NumPy 2.4.6,
and the INT8 Parakeet TDT 0.6B v2 model. Ordinary Faithful dictation needs no
separate Python installation or speech-model download. The model is the English
Parakeet v2 model; do not infer multilingual recognition from the app's platform
support. Whisper and Ollama are not included in these Unix packages.

The bundle manifest records source revision, download hashes and installed-file
hashes. Dependency and model license notices are included. Initial downloads are
large because the runtime and model are included.

## macOS

Use the Apple Silicon (`macos-arm64`) package on an Apple Silicon Mac, or
`macos-x64` on an Intel Mac. The initial supported package target is macOS 15 or
newer. The native CI build runs on macOS 15; other OS versions and applications
require their own interactive verification.

Open the `.pkg` installer to install `/Applications/Parla.app`, then open Parla
from Applications. Grant microphone access when requested. Global hotkeys and
automatic typing can also need Accessibility and Input Monitoring permissions
under System Settings -> Privacy & Security. Review the requested application
identity and reopen Parla after changing permissions. The package does not grant
these permissions automatically.

These initial packages are **not Developer ID signed or notarized**. Individual
native binaries receive ad-hoc signatures where needed; this does not establish
a verified publisher or bypass Gatekeeper. macOS may block installation or launch.
Use Apple's per-application Privacy & Security review after checking the source
and checksum; do not disable Gatekeeper globally. A managed Mac may disallow it.

To inspect setup without opening a microphone or typing:

```sh
/Applications/Parla.app/Contents/MacOS/parla --check-setup
```

The payload is under `Parla.app/Contents/Resources`; the executable is under
`Contents/MacOS`. Keep the app together. Do not move just the executable.
Remove the app from Applications to uninstall the bundled program and models;
user settings/history are separate and are preserved.

## Linux x64

The initial build baseline is Ubuntu 22.04 x64 / glibc 2.35. The Debian package
declares its audio and X11 runtime dependencies. This is not an Alpine/musl or
Linux ARM build.

For Debian/Ubuntu, install the downloaded package with the distribution's package
manager so dependencies are resolved:

```sh
sudo apt install ./Parla-0.3.0-portable-20260921-linux-x64.deb
parla --check-setup
parla --dashboard
```

The package installs application files under `/opt/parla`, a command under
`/usr/bin/parla`, and a desktop application entry. Start it through the desktop
entry or with `parla --dashboard`. User settings and history remain outside
`/opt/parla`.

Alternatively, extract the `.tar.gz` into a writable directory and run its
`parla-launch` script. Keep `parla`, `models`, `runtime` and the manifest together.
The archive is relocatable but still requires the documented system libraries:
ALSA, X11, Xtst, xdo, xcb, xkbcommon, glibc and libstdc++.

X11 is the initial automatic-hotkey/insertion target. On Wayland, use the local
dashboard's Start/stop controls and Copy final, then paste manually. You can bind
`parla toggle` through your desktop's own shortcut settings. This does not supply
a native global shortcut or automatic insertion mechanism. Review the release's
exact desktop support matrix before relying on hands-free use. Clipboard fallback
is not the same as verified automatic typing.

Remove the Debian package with `sudo apt remove parla`; user settings/history are
preserved. For the archive, remove only the directory you extracted.

## Verification scope

The packaging workflow runs Rust tests, constructs real native packages, installs
them on the runner, verifies the payload hashes, imports the packaged Python
dependencies, initializes isolated settings, verifies that initialization preserves
existing settings, exports the formatter prompt, and replays the public speech
fixture from the pinned model archive through the installed executable. Linux
also tests a freshly extracted, relocated tarball.

The replay checks two expected transcript phrases and records its actual output.
It does not record a microphone, inject keyboard input, or test interactive
permission prompts, global hotkeys, every application, or every desktop.
Installer success and recognition success must not be described as those broader
compatibility guarantees.

## Rebuild

The GitHub `Portable offline packages` workflow is manually dispatched or triggered
by relevant pushes to `main` / `feat/portable-speech-20260921` for full packages
on standard native runners: `macos-15`, `macos-15-intel`, and
`ubuntu-22.04`. It uploads build artifacts and evidence; it does not publish a
GitHub release. Pull requests run the tests/build without large model packaging.

On the equivalent native build host, using Rust 1.98.0 and Python 3.12 or newer:

```sh
cargo test --locked --workspace
cargo build --locked --release --workspace
python3 scripts/package-unix.py --self-test
python3 scripts/package-unix.py --target linux-x64 --output release/my-linux-build
```

Substitute `macos-arm64` or `macos-x64` on the matching Mac. Packaging invokes
`sudo installer` or `sudo dpkg` for an installed-payload test, so use a disposable
build machine. The script refuses to overwrite an existing output directory.
Pinned inputs and normalized Linux archive metadata make the assembly repeatable;
native compiler/SDK and Apple package metadata mean byte-for-byte reproducibility
of all outputs is not claimed.

Primary distribution references: [Python Build Standalone pinned checksums](https://github.com/astral-sh/python-build-standalone/releases/download/20260901/SHA256SUMS),
[sherpa-onnx wheel metadata](https://pypi.org/pypi/sherpa-onnx/1.13.7/json),
[NumPy wheel metadata](https://pypi.org/pypi/numpy/2.4.6/json), and
[GitHub runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
