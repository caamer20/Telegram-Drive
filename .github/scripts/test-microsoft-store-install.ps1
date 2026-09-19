[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$PackagePath,
  [Parameter(Mandatory = $true)][string]$ReportDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'This installation test is restricted to disposable GitHub-hosted Windows runners.'
}
$PackagePath = [IO.Path]::GetFullPath($PackagePath)
$ReportDirectory = [IO.Path]::GetFullPath($ReportDirectory)
New-Item -ItemType Directory -Path $ReportDirectory -Force | Out-Null
$packageName = 'CameronAmer.Telegram-Drive'
$publisher = 'CN=89634FAE-7570-4D3F-B685-647564F5B488'
$family = 'CameronAmer.Telegram-Drive_38vgcacrksbrt'
$receipt = Get-Content "$PackagePath.json" -Raw | ConvertFrom-Json
$originalHash = (Get-FileHash $PackagePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($receipt.packageSha256 -ne $originalHash -or $receipt.packageName -ne $packageName -or $receipt.publisher -ne $publisher) {
  throw 'The package does not match its expected Store identity and checksum receipt.'
}
if (Get-AppxPackage -Name $packageName) { throw 'Refusing to modify an existing installation.' }
$dataDirectories = @(
  (Join-Path $env:LOCALAPPDATA 'com.cameronamer.telegramdrive'),
  (Join-Path $env:APPDATA 'com.cameronamer.telegramdrive')
)
foreach ($directory in $dataDirectories) {
  if (Test-Path -LiteralPath $directory) { throw 'Refusing to touch existing application data.' }
}
$testId = [guid]::NewGuid().ToString('N')
$testRoot = Join-Path $env:RUNNER_TEMP "telegram-drive-store-$testId"
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$report = [ordered]@{
  packageSha256 = $originalHash
  os = (Get-CimInstance Win32_OperatingSystem).Caption
  installation = 'pending'
  officialIcons = 'pending'
  packagedDataAndCredentialProbe = 'pending'
  uninstallPreservesUnvirtualizedData = 'pending'
  desktopUiAndAuthenticatedTelegramAcceptance = 'not tested; requires Windows 10/11 desktop validation'
  storeCertification = 'not performed'
}
$certificate = $null
$installed = $false
$credentialTarget = "store-validation-$testId.com.cameronamer.telegramdrive.supporter"
$markerPaths = @()
try {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $unpacked = Join-Path $testRoot 'unpacked'
  [IO.Compression.ZipFile]::ExtractToDirectory($PackagePath, $unpacked)
  foreach ($asset in @('StoreLogo.png', 'Square44x44Logo.png', 'Square150x150Logo.png')) {
    $source = Join-Path $PSScriptRoot "../../app/src-tauri/icons/$asset"
    if ((Get-FileHash (Join-Path $unpacked "Assets/$asset")).Hash -ne (Get-FileHash $source).Hash) {
      throw "The package icon differs from the official release icon: $asset"
    }
  }
  $report.officialIcons = 'passed: all packaged Windows logos match the released assets'
  $signTool = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Filter signtool.exe -File -Recurse |
    Where-Object { $_.Directory.Name -eq 'x64' -and $_.Directory.Parent.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
  if (-not $signTool) { throw 'Windows SDK SignTool is unavailable.' }
  $certificate = New-SelfSignedCertificate -Type Custom -KeyUsage DigitalSignature -CertStoreLocation Cert:\CurrentUser\My `
    -Subject $publisher -FriendlyName "Disposable Telegram-Drive CI $testId" -NotAfter (Get-Date).AddHours(3) `
    -KeyExportPolicy NonExportable -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
  $publicCertificate = Join-Path $testRoot 'test-only.cer'
  Export-Certificate -Cert $certificate -FilePath $publicCertificate | Out-Null
  Import-Certificate -FilePath $publicCertificate -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
  $signedCopy = Join-Path $testRoot 'test-only.msix'
  Copy-Item -LiteralPath $PackagePath -Destination $signedCopy
  & $signTool.FullName sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $signedCopy
  if ($LASTEXITCODE -ne 0) { throw 'Test-only package signing failed.' }
  $vcLibs = Get-ChildItem "${env:ProgramFiles(x86)}/Microsoft SDKs/Windows Kits/10/ExtensionSDKs/Microsoft.VCLibs.Desktop/14.0" -Filter '*x64*.appx' -Recurse -File |
    Where-Object { $_.FullName -notmatch '(?i)[\\/]debug[\\/]' } | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
  if (-not $vcLibs) { throw 'The official VCLibs Desktop dependency is unavailable.' }
  foreach ($directory in $dataDirectories) {
    New-Item -ItemType Directory -Path $directory | Out-Null
    $marker = Join-Path $directory "store-validation-$testId.txt"
    [IO.File]::WriteAllText($marker, $testId)
    $markerPaths += $marker
  }
  & cmdkey.exe "/generic:$credentialTarget" '/user:store-validation-fixture' "/pass:$testId" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'Could not create the disposable credential fixture.' }
  Add-AppxPackage -Path $signedCopy -DependencyPath $vcLibs.FullName
  $installed = $true
  $package = Get-AppxPackage -Name $packageName
  if ($package.PackageFamilyName -ne $family -or $package.Publisher -ne $publisher) { throw 'Installed package identity differs.' }
  $report.installation = 'passed: signed test copy installed with the expected Store identity and dependency'
  $probeConfig = Join-Path $testRoot 'probe-config.json'
  $probeOutput = Join-Path $testRoot 'probe-result.json'
  @{ testId = $testId; target = $credentialTarget; markers = $markerPaths; output = $probeOutput } | ConvertTo-Json | Set-Content $probeConfig
  $probeScript = Join-Path $testRoot 'probe.ps1'
  @'
param([string]$ConfigPath)
$ErrorActionPreference = 'Stop'
$config = Get-Content $ConfigPath -Raw | ConvertFrom-Json
$result = @{ passed = $false }
try {
  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class StoreProbe {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
  public static extern int GetCurrentPackageFullName(ref int length, StringBuilder value);
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  public struct Credential {
    public int Flags, Type; public string TargetName, Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public int CredentialBlobSize; public IntPtr CredentialBlob;
    public int Persist, AttributeCount; public IntPtr Attributes;
    public string TargetAlias, UserName;
  }
  [DllImport("advapi32.dll", EntryPoint="CredReadW", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool CredRead(string target, int type, int flags, out IntPtr credential);
  [DllImport("advapi32.dll")] public static extern void CredFree(IntPtr buffer);
  public static string ReadFixture(string target) {
    IntPtr ptr;
    if (!CredRead(target, 1, 0, out ptr)) throw new Exception("Credential fixture is inaccessible");
    try {
      var cred = (Credential)Marshal.PtrToStructure(ptr, typeof(Credential));
      return Marshal.PtrToStringUni(cred.CredentialBlob, cred.CredentialBlobSize / 2);
    } finally { CredFree(ptr); }
  }
}
"@
  $length = 0
  $code = [StoreProbe]::GetCurrentPackageFullName([ref]$length, $null)
  if ($code -ne 122) { throw 'The probe has no package identity.' }
  $name = New-Object Text.StringBuilder $length
  if ([StoreProbe]::GetCurrentPackageFullName([ref]$length, $name) -ne 0 -or $name.ToString() -notlike 'CameronAmer.Telegram-Drive_*') {
    throw 'Unexpected probe package identity.'
  }
  foreach ($marker in $config.markers) {
    if ([IO.File]::ReadAllText($marker) -ne $config.testId) { throw 'Existing data is inaccessible from the package.' }
    [IO.File]::WriteAllText($marker, $config.testId + '-packaged')
  }
  if ([StoreProbe]::ReadFixture($config.target) -ne $config.testId) { throw 'Existing credential fixture changed.' }
  $result = @{ passed = $true; packageIdentity = $name.ToString(); sharedAppData = $true; existingCredentialVisible = $true }
} catch { $result.error = $_.Exception.Message }
$result | ConvertTo-Json | Set-Content $config.output
'@ | Set-Content $probeScript
  Invoke-CommandInDesktopPackage -PackageFamilyName $family -AppId TelegramDrive -Command "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" `
    -Args "-NoProfile -NonInteractive -File `"$probeScript`" -ConfigPath `"$probeConfig`"" -PreventBreakaway
  $deadline = (Get-Date).AddSeconds(60)
  while (-not (Test-Path $probeOutput) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
  if (-not (Test-Path $probeOutput)) { throw 'The packaged compatibility probe did not finish.' }
  $probe = Get-Content $probeOutput -Raw | ConvertFrom-Json
  if (-not $probe.passed) { throw "Packaged compatibility probe failed: $($probe.error)" }
  foreach ($marker in $markerPaths) {
    if ([IO.File]::ReadAllText($marker) -ne "$testId-packaged") { throw 'Packaged writes were redirected away from existing app data.' }
  }
  $report.packagedDataAndCredentialProbe = $probe
  Remove-AppxPackage -Package $package.PackageFullName
  $installed = $false
  foreach ($marker in $markerPaths) {
    if ([IO.File]::ReadAllText($marker) -ne "$testId-packaged") { throw 'Uninstall removed unvirtualized app data.' }
  }
  $report.uninstallPreservesUnvirtualizedData = 'passed'
  if ((Get-FileHash $PackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $originalHash) { throw 'The unsigned Store candidate was modified.' }
  $report.passed = $true
} catch {
  $report.passed = $false
  $report.error = $_.Exception.Message
  throw
} finally {
  if ($installed) { Get-AppxPackage -Name $packageName | Remove-AppxPackage -ErrorAction Continue }
  & cmdkey.exe "/delete:$credentialTarget" | Out-Null
  foreach ($directory in $dataDirectories) { if (Test-Path $directory) { Remove-Item -LiteralPath $directory -Recurse -Force } }
  if ($certificate) {
    Remove-Item -LiteralPath "Cert:\LocalMachine\TrustedPeople\$($certificate.Thumbprint)" -Force -ErrorAction Continue
    Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -DeleteKey -Force -ErrorAction Continue
  }
  $report | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $ReportDirectory 'installation-result.json')
  Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Continue
}
