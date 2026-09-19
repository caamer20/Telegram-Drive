# Microsoft Store submission

Status: preparation in progress. No Store package has been built, uploaded, certified, or published yet.

## Distribution choice

The repository owner selected MSIX with Microsoft Store signing on September 18, 2026. Microsoft signs an accepted MSIX during publication; purchasing a Windows code-signing certificate is not required for this route.

The published v3.9.0 Windows NSIS installer was downloaded and its SHA-256 verified against the release checksum file:

```text
Telegram.Drive_3.9.0_x64-setup.exe
33203791 bytes
aac06c3c15135642a9fd64653ab105362ec8a41ee312337dbe216fe94343d512
```

It contains an unsigned `app.exe` and an online WebView2 bootstrapper. Its detached `.sig` is a Tauri updater signature, not Windows Authenticode. It cannot be submitted unchanged as a Store EXE. Simply wrapping its application binary in MSIX would retain the standalone GitHub updater, so a Store-compatible build is required.

All Windows preparation is based on the published desktop source in an isolated checkout. Local Android project/source changes must never be included in GitHub commits, CI source uploads, or Store uploads. The MSIX payload is explicitly limited to the compiled Windows application, the Microsoft runtime, the app manifest and existing Windows logo assets.

## Account and identity

Developer enrollment is complete. The owner chose **Telegram-Drive**, which was reserved successfully in Partner Center on September 19, 2026. The original unhyphenated name was unavailable.

Verified product identity:

- Store ID: `9PM592MZ4PF1`
- Package/Identity/Name: `CameronAmer.Telegram-Drive`
- Package/Identity/Publisher: `CN=89634FAE-7570-4D3F-B685-647564F5B488`
- PublisherDisplayName: `CameronAmer`
- Package family: `CameronAmer.Telegram-Drive_38vgcacrksbrt`
- Future listing URL: https://apps.microsoft.com/detail/9PM592MZ4PF1 (not published yet)

These public package values are separate from the unchanged Tauri application and credential identifiers. Automatic approval review blocked starting the Store submission while MSIX build and native validation remain pending. Complete those checks before retrying.

## Windows build

Use PowerShell 7 on Windows with Node, Rust, the Visual C++ build tools, Windows SDK and the retail x64 VCLibs Desktop Bridge framework package. Install the frontend dependencies in the isolated checkout.

1. Obtain the current official **x64 Fixed Version WebView2 Runtime** from Microsoft. Extract it into `app/src-tauri/resources/webview2`, with `msedgewebview2.exe` directly in that folder. Keep this downloaded runtime out of Git. Record its source URL, version and checksum in the release record. The packaging script checks its Microsoft Authenticode signature.
2. Supply the existing production values for `TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL` and `TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY`, unchanged from the desktop release. Do not create or rotate any supporter key.
3. Run the protected supporter checks in `SUPPORTER_LICENSE_INVARIANTS.md` and the frontend build/tests.
4. Build and package using the actual Partner Center identity:

```powershell
pwsh -File app/scripts/package-microsoft-store.ps1 `
  -PackageName 'CameronAmer.Telegram-Drive' `
  -Publisher 'CN=89634FAE-7570-4D3F-B685-647564F5B488' `
  -PublisherDisplayName 'CameronAmer'
```

If the Windows SDK's VCLibs package is not discoverable, supply `-VcLibsPackage` with the official x64 retail `Microsoft.VCLibs.140.00.UWPDesktop` appx. The package manifest uses the version and publisher from that framework package, which Microsoft Store installs as a dependency. Do not include or run `vc_redist.x64.exe` inside the MSIX.

The script builds the application with the Store configuration, includes the fixed WebView2 runtime, and invokes MakeAppx with manifest validation enabled. The candidate and a checksum/validation-status receipt are written under the ignored `CompiledApps/MicrosoftStore` directory. The fourth MSIX version component is zero. No signing key or application source is packaged.

The fixed WebView2 runtime needs to be refreshed and tested for later Store releases; it does not receive Evergreen runtime updates. The initial package targets x64 Windows 10 build 19041 or later. Native compatibility remains to be verified before submission.

## Branding and public links

The owner confirmed the released Telegram-Drive icon and these two public sites on September 19, 2026:

- Product website: https://telegram-drive.com
- Developer website: https://www.cameronamer.com

Use the existing `app/src-tauri/icons/icon.png` and Windows logo variants. The main icon SHA-256 is `208ac26fcff2a05818b934c0db98e4a36f5d91bca6761efc8fc01f1a03e1820a`; the isolated checkout matches the official released and current local assets. Include both sites in the description if Partner Center provides only one website field, with the product site in that field.

## Approved runtime

The owner explicitly approved accepting and downloading Microsoft Edge WebView2 Fixed Version 153.0.4234.48 x64. The Microsoft-hosted CAB was downloaded after acceptance and has SHA-256 `11e8240cb0bc56dcd3e4498907203c251346f65107fe35a3a13e152c7d51c79e` (308,509,880 bytes). Its official source URL and checksum are supplied through the Store workflow repository variables. Windows Authenticode verification remains mandatory in the packaging script.

Keep all runtime notices in the package. The Store listing must disclose the runtime's Microsoft Defender SmartScreen data handling, with Microsoft's privacy links, and the required runtime terms must be included in the submission's license information before publication. This does not alter the application's optional $5 lifetime supporter contract.

## Updates and existing users

- A Windows package-identity check identifies MSIX installations. The standalone updater plugin is not registered for those installations; the frontend also avoids the GitHub update check. The Settings update action opens Microsoft's supported Store updates page.
- Unpackaged Windows installations retain their existing signed Tauri updater. Linux package-manager behavior is unchanged.
- Keep the Tauri identifier `com.cameronamer.telegramdrive` and the secure credential service `com.cameronamer.telegramdrive.supporter` unchanged. The Microsoft-assigned package identity is separate from those existing storage identifiers.
- The MSIX manifest preserves the existing application data location. Windows 11 exclusions are limited to the app's LocalAppData and RoamingAppData directories; the older Windows manifest property preserves the same location on supported Windows 10 builds. This requires Microsoft's review of `unvirtualizedResources`.
- No entitlement, price, token format, device allowance, recovery code, signing key, credential account, or database migration is introduced. One verified $5.00 USD payment still grants lifetime ad-free use on up to three devices, and every feature remains free.
- Do not delete or overwrite a prior installation's data. Before recommending migration from the standalone installer, test supporter activation, device identity, offline grace and recovery on an isolated Windows installation. Do not assume that successful packaging proves credential compatibility.

## Native acceptance still required

Use an isolated Windows VM and test-only credentials. A local test certificate may be used for sideload validation; do not ship its private key or install it on a customer's machine.

- Fresh installation and launch, including a machine without an existing WebView2 runtime; verify the packaged runtime is used.
- Actual file upload/download, previews and playback; tray behavior, single-instance handling, notifications, optional local REST/WebDAV sharing and keyboard input.
- Existing standalone-to-Store activation and data preservation, Store-to-Store updates, offline-grace ad suppression, recovery and device allowance. All stable credential identifiers must remain unchanged.
- Store-managed updates never launch the GitHub installer, including Settings and background update checks.
- Windows App Certification Kit, dependency resolution, package manifest validation and installation on supported Windows 10/11 versions.
- The owner explicitly requested the existing project screenshots. Use the selected original desktop images unchanged; they include macOS window chrome and must not be described as newly captured Windows screenshots. Microsoft may request platform-specific replacements during review.

## Store submission

Prepare an English listing in Productivity, with a free base price and clear disclosure of the optional one-time $5.00 USD lifetime ad-free purchase and sponsor advertisements. Every feature remains available without payment. Preserve the existing privacy policy and supporter terms links.

The description must disclose the required Telegram account, Telegram API ID/API hash and network access. Telegram Drive is independent and not affiliated with Telegram FZ-LLC. Do not advertise unlimited storage or audited encryption.

Provide a dedicated, owner-approved way for Microsoft certification reviewers to test authenticated Telegram functionality. Never submit the owner's personal Telegram credentials, recovery codes, signing material or private file data. Review account access and authentication instructions are still pending.

Complete pricing/availability, properties, the actual age-rating questionnaire, package upload, listing screenshots and certification notes. Explain `runFullTrust` and `unvirtualizedResources` with the concrete file-workspace and existing-data compatibility requirements. Upload validation, certification approval and public availability are separate stages; report the actual stage.

## Preparation checks completed on macOS

- Frontend unit suite, including the protected supporter UI tests: passed.
- Frontend TypeScript/build and all bundle budgets: passed.
- Store update routing, Settings and existing release gates: 28 focused tests passed.
- Native installation classification: three tests passed.
- Native supporter compatibility: 21 tests passed.
- Supporter service TypeScript check and 36 unit tests: passed.
- Supporter Worker bundle dry run: passed; no production deployment performed.
- PowerShell packaging script: syntax parsed successfully with official PowerShell 7.

GitHub Actions run `35423391333`, attempt 1, also passed the focused frontend checks/build, native Windows installation and supporter tests, and supporter-service checks on Windows Server 2022. The MSIX job was skipped while runtime approval was pending. Those checks do not prove installed-package behavior, preserve Windows credentials across a real installation, or replace Windows 10/11 acceptance and Store certification.

## Official references

- [MSIX package requirements and Store signing](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)
- [EXE/MSI requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements)
- [MSIX submission checklist](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/create-app-submission)
- [Windows app packaging preparation](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare)
- [Application data virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization)
- [Desktop Bridge C++ runtime dependency](https://learn.microsoft.com/en-us/troubleshoot/developer/visualstudio/cpp/libraries/c-runtime-packages-desktop-bridge)
- [WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)
- [Microsoft Store update-page URI](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-store-app)
