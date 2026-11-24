type Translate = (key: string, options?: Record<string, unknown>) => string;

const SAFE_ERROR_KEYS: Record<string, string> = {
  FOLDER_OFFLINE: 'workspace.error_network',
  VAULT_LOCKED: 'settings.vault_is_locked',
  ENCRYPTED_SHARE_UNAVAILABLE: 'common.operation_failed',
  ENCRYPTED_PREVIEW_UNAVAILABLE: 'archive.failed_read',
  WRONG_KEY_OR_CORRUPT: 'common.operation_failed',
};

function serializedError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === 'string' ? error : '';
}

/**
 * Converts backend failures into reviewed localized copy. Unknown error text is
 * deliberately discarded because it may contain paths, URLs, IDs, or secrets.
 */
export function userFacingError(error: unknown, t: Translate): string {
  const raw = serializedError(error);
  const code = raw.match(/^\[([A-Z0-9_]+)]/)?.[1];
  if (code && SAFE_ERROR_KEYS[code]) return t(SAFE_ERROR_KEYS[code]);

  const floodWait = raw.match(/FLOOD_WAIT_?(\d+)/i)?.[1];
  if (floodWait) {
    return t('auth.flood_wait_msg', { seconds: Number(floodWait) });
  }

  return t('common.operation_failed');
}

/** Native crypto errors retain both legacy CamelCase and wire-code spellings. */
export function recoveryError(error: unknown, t: Translate): string {
  const message = serializedError(error);
  const code = message.match(/^\[([a-z_]+)\]/i)?.[1].replace(/_/g, '').toLowerCase();
  switch (code) {
    case 'wrongkeyorcorrupt':
    case 'authfailed':
    case 'headerinvalid':
    case 'truncated':
    case 'kdflimitexceeded':
      return t('action_errors.verify_bundle');
    case 'vaultlocked':
      return t('settings.vault_is_locked');
    case 'ioerror':
      return t('activity.category_storage');
    case 'unsupportedversion':
      return t('settings.check_for_updates');
    default:
      return userFacingError(error, t);
  }
}
