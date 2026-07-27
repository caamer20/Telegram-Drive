param(
    [Parameter(Mandatory)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$ApkPath
)

$ErrorActionPreference = 'Stop'
$readElf = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Android\Sdk\ndk') `
    -Filter llvm-readelf.exe -Recurse |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName

if (-not $readElf) {
    throw 'llvm-readelf.exe was not found in the Android NDK.'
}

$workDir = Join-Path ([System.IO.Path]::GetTempPath()) ('telegram-drive-elf-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workDir | Out-Null

try {
    tar -xf $ApkPath -C $workDir 'lib/arm64-v8a/libapp_lib.so'
    if ($LASTEXITCODE -ne 0) {
        throw 'The APK does not contain lib/arm64-v8a/libapp_lib.so.'
    }

    $library = Join-Path $workDir 'lib\arm64-v8a\libapp_lib.so'
    $symbols = & $readElf -Ws $library
    $needed = & $readElf -d $library
    if ($LASTEXITCODE -ne 0) {
        throw 'llvm-readelf could not inspect libapp_lib.so.'
    }

    if ($symbols -match 'UND _ZTISt12length_error' -and
        $needed -notmatch 'Shared library: \[libc\+\+_shared\.so\]') {
        throw 'libapp_lib.so uses the C++ runtime without declaring libc++_shared.so.'
    }
} finally {
    Remove-Item -LiteralPath $workDir -Recurse -Force
}

Write-Host 'PASS: libapp_lib.so has no unresolved C++ runtime dependency.'
