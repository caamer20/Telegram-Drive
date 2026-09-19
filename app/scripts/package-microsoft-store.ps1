[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[A-Za-z0-9.-]{3,50}$')]
  [string]$PackageName,
  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$Publisher,
  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$PublisherDisplayName,
  [string]$AppRoot = (Join-Path $PSScriptRoot '..'),
  [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../CompiledApps/MicrosoftStore'),
  [string]$VcLibsPackage
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'MSIX build and validation must run on Windows with the Windows SDK.' }
$AppRoot = [IO.Path]::GetFullPath($AppRoot)
$tauriRoot = Join-Path $AppRoot 'src-tauri'
$runtime = Join-Path $tauriRoot 'resources/webview2'
$runtimeExe = Join-Path $runtime 'msedgewebview2.exe'
if (-not (Test-Path -LiteralPath $runtimeExe -PathType Leaf)) {
  throw 'Extract the official x64 WebView2 Fixed Version runtime into app/src-tauri/resources/webview2 first. The directory must directly contain msedgewebview2.exe.'
}
$signature = Get-AuthenticodeSignature -FilePath $runtimeExe
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
  throw 'The fixed WebView2 runtime must have a valid Microsoft Authenticode signature.'
}
if ($Publisher -notmatch '^CN=') { throw 'Use the exact Package/Identity/Publisher value from Partner Center.' }
if ($env:TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL -notmatch '^https://[^/]+$' -or
    $env:TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY -notmatch '^[A-Za-z0-9_-]{43}$') {
  throw 'Provide the unchanged production supporter HTTPS origin and public verification key. Do not create a new signing key for the Store.'
}

$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$makeAppx = Get-ChildItem $sdkRoot -Filter makeappx.exe -Recurse -File |
  Where-Object { $_.Directory.Name -eq 'x64' -and $_.Directory.Parent.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
  Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
  Select-Object -First 1
if (-not $makeAppx) { throw 'Install the Windows 10/11 SDK containing x64 MakeAppx.exe.' }

if (-not $VcLibsPackage) {
  $sdkExtensions = Join-Path ${env:ProgramFiles(x86)} 'Microsoft SDKs/Windows Kits/10/ExtensionSDKs/Microsoft.VCLibs.Desktop/14.0'
  $package = Get-ChildItem $sdkExtensions -Filter '*x64*.appx' -Recurse -File |
    Where-Object { $_.FullName -notmatch '(?i)[\\/]debug[\\/]' } |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
  if (-not $package) { throw 'Provide -VcLibsPackage with the Microsoft x64 retail Desktop Bridge C++ runtime framework package.' }
  $VcLibsPackage = $package.FullName
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($VcLibsPackage))
try {
  $entry = $zip.GetEntry('AppxManifest.xml')
  if (-not $entry) { throw 'VCLibs framework package has no manifest.' }
  $reader = [IO.StreamReader]::new($entry.Open())
  try { [xml]$vcManifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
} finally { $zip.Dispose() }
$vcIdentity = $vcManifest.Package.Identity
$microsoftPublisher = 'CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US'
if ($vcIdentity.Name -ne 'Microsoft.VCLibs.140.00.UWPDesktop' -or
    $vcIdentity.Publisher -ne $microsoftPublisher -or
    $vcIdentity.ProcessorArchitecture -ne 'x64') {
  throw 'Expected the Microsoft-supplied x64 retail VCLibs Desktop Bridge package.'
}

Push-Location $AppRoot
try {
  # The regular npm wrapper prepares the standalone NSIS prerequisites.
  # MSIX uses the VCLibs framework and the verified fixed WebView2 runtime.
  & (Join-Path $AppRoot 'node_modules/.bin/tauri.cmd') build --no-bundle --config src-tauri/tauri.microsoft-store.conf.json
  if ($LASTEXITCODE -ne 0) { throw 'The Store build failed.' }
} finally { Pop-Location }

$version = (Get-Content (Join-Path $AppRoot 'package.json') -Raw | ConvertFrom-Json).version
if ($version -notmatch '^([1-9][0-9]*)\.([0-9]+)\.([0-9]+)$') {
  throw 'A stable three-component application version is required for Store packaging.'
}
$packageVersion = "$version.0"
foreach ($component in $packageVersion.Split('.')) {
  if ([int]$component -gt 65535) { throw 'Each MSIX version component must fit in 16 bits.' }
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$payload = Join-Path $OutputDirectory ('payload-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $payload 'Assets'), (Join-Path $payload 'resources') -Force | Out-Null
$application = Join-Path $tauriRoot 'target/release/app.exe'
if (-not (Test-Path -LiteralPath $application -PathType Leaf)) { throw 'The compiled Windows app.exe is missing.' }
Copy-Item -LiteralPath $application -Destination (Join-Path $payload 'app.exe')
Copy-Item -LiteralPath $runtime -Destination (Join-Path $payload 'resources/webview2') -Recurse
foreach ($asset in @('StoreLogo.png', 'Square44x44Logo.png', 'Square150x150Logo.png')) {
  Copy-Item -LiteralPath (Join-Path $tauriRoot "icons/$asset") -Destination (Join-Path $payload "Assets/$asset")
}
function Escape-Xml([string]$value) { [Security.SecurityElement]::Escape($value) }
$nameXml = Escape-Xml $PackageName
$publisherXml = Escape-Xml $Publisher
$displayXml = Escape-Xml $PublisherDisplayName
$vcVersion = Escape-Xml $vcIdentity.Version
$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
 xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
 xmlns:desktop6="http://schemas.microsoft.com/appx/manifest/desktop/windows10/6"
 xmlns:virtualization="http://schemas.microsoft.com/appx/manifest/virtualization/windows10"
 xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
 IgnorableNamespaces="uap desktop6 virtualization rescap">
 <Identity Name="$nameXml" Publisher="$publisherXml" Version="$packageVersion" ProcessorArchitecture="x64" />
 <Properties>
  <DisplayName>Telegram-Drive</DisplayName>
  <PublisherDisplayName>$displayXml</PublisherDisplayName>
  <Logo>Assets\StoreLogo.png</Logo>
  <desktop6:FileSystemWriteVirtualization>disabled</desktop6:FileSystemWriteVirtualization>
  <virtualization:FileSystemWriteVirtualization>
   <virtualization:ExcludedDirectories>
    <virtualization:ExcludedDirectory>`$(KnownFolder:LocalAppData)\com.cameronamer.telegramdrive</virtualization:ExcludedDirectory>
    <virtualization:ExcludedDirectory>`$(KnownFolder:RoamingAppData)\com.cameronamer.telegramdrive</virtualization:ExcludedDirectory>
   </virtualization:ExcludedDirectories>
  </virtualization:FileSystemWriteVirtualization>
 </Properties>
 <Resources><Resource Language="en-us" /></Resources>
 <Dependencies>
  <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0" />
  <PackageDependency Name="Microsoft.VCLibs.140.00.UWPDesktop" MinVersion="$vcVersion" Publisher="$microsoftPublisher" />
 </Dependencies>
 <Applications>
  <Application Id="TelegramDrive" Executable="app.exe" EntryPoint="Windows.FullTrustApplication">
   <uap:VisualElements DisplayName="Telegram-Drive" Description="A file workspace for your Telegram files."
    Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png" BackgroundColor="transparent" />
  </Application>
 </Applications>
 <Capabilities>
  <rescap:Capability Name="runFullTrust" />
  <rescap:Capability Name="unvirtualizedResources" />
 </Capabilities>
</Package>
"@
[IO.File]::WriteAllText((Join-Path $payload 'AppxManifest.xml'), $manifest, [Text.UTF8Encoding]::new($false))
$output = Join-Path $OutputDirectory "Telegram-Drive_${packageVersion}_x64.msix"
if (Test-Path -LiteralPath $output) { throw "Refusing to overwrite an existing package: $output" }
& $makeAppx.FullName pack /d $payload /p $output /h SHA256
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx packaging or manifest validation failed.' }
$receipt = [ordered]@{
  package = [IO.Path]::GetFileName($output)
  packageSha256 = (Get-FileHash $output -Algorithm SHA256).Hash.ToLowerInvariant()
  applicationSha256 = (Get-FileHash $application -Algorithm SHA256).Hash.ToLowerInvariant()
  version = $packageVersion
  architecture = 'x64'
  packageName = $PackageName
  publisher = $Publisher
  webView2Version = (Get-Item $runtimeExe).VersionInfo.FileVersion
  vcLibsVersion = $vcIdentity.Version
  signed = $false
  storeSigning = 'Microsoft signs after certification'
  nativeAcceptance = 'PENDING: run Windows installation, upgrade, activation and certification checks before submission'
}
$receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$output.json" -Encoding utf8
Write-Host "Unsigned Store candidate created: $output"
Write-Host 'Complete the Windows acceptance checks before uploading for certification. No source or keys are included.'
