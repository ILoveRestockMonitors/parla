param([Parameter(Mandatory=$true)][string]$CompilerBin)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$source=Join-Path $root 'release/whisper-source'
$build=Join-Path $root 'release/whisper-build'
$revision='48f628a84833905ee4a0658ee6d4a5c915ce1997'
if(!(Test-Path -LiteralPath $source)){
    git clone --depth 1 --branch v1.8.7 https://github.com/ggml-org/whisper.cpp.git $source
    if($LASTEXITCODE -ne 0){throw 'Whisper source clone failed'}
}
if((git -C $source rev-parse HEAD) -ne $revision){throw 'Unexpected Whisper source revision'}
if(git -C $source status --porcelain){throw 'Whisper source has local changes'}
$compiler=(Resolve-Path -LiteralPath $CompilerBin).Path
$env:PATH=$compiler+';'+$env:PATH
& (Join-Path $compiler 'cmake.exe') -S $source -B $build -G Ninja '-DCMAKE_BUILD_TYPE=Release' '-DBUILD_SHARED_LIBS=OFF' '-DGGML_NATIVE=OFF' '-DGGML_AVX=OFF' '-DGGML_AVX2=OFF' '-DGGML_FMA=OFF' '-DGGML_F16C=OFF' '-DGGML_OPENMP=OFF' '-DWHISPER_BUILD_TESTS=OFF' '-DWHISPER_BUILD_SERVER=ON' '-DCMAKE_EXE_LINKER_FLAGS=-static -static-libgcc -static-libstdc++'
if($LASTEXITCODE -ne 0){throw 'Whisper configure failed'}
& (Join-Path $compiler 'cmake.exe') --build $build --target whisper-server -j 8
if($LASTEXITCODE -ne 0){throw 'Whisper build failed'}
$imports=& (Join-Path $compiler 'objdump.exe') -p (Join-Path $build 'bin/whisper-server.exe')
if($imports -match 'DLL Name:.*(MSVCP|VCRUNTIME|VCOMP|libgcc|libstdc|libwinpthread)'){throw 'Whisper still requires an external compiler runtime'}
Get-FileHash -LiteralPath (Join-Path $build 'bin/whisper-server.exe') -Algorithm SHA256
