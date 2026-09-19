import { invoke } from '@tauri-apps/api/core';

export interface InstallationInfo {
    managedByPackageManager: boolean;
    packageManager: 'pacman' | 'microsoft-store' | null;
}

export const RELEASES_URL = 'https://github.com/caamer20/Telegram-Drive/releases/latest';

const SELF_MANAGED_INSTALLATION: InstallationInfo = {
    managedByPackageManager: false,
    packageManager: null,
};

let installationInfoPromise: Promise<InstallationInfo> | null = null;

export function getInstallationInfo(): Promise<InstallationInfo> {
    if (!installationInfoPromise) {
        installationInfoPromise = invoke<InstallationInfo>('cmd_get_installation_info')
            .catch(() => SELF_MANAGED_INSTALLATION);
    }
    return installationInfoPromise;
}
