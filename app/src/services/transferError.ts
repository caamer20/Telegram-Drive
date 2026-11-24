import type { TransferErrorCategory } from './transferActivity';

type Translate = (key: string, values?: Record<string, unknown>) => string;
interface Failure { error?: string; errorCategory?: TransferErrorCategory | null }

// Older queue entries and transfers running before native persistence can carry
// only a message. Match known reasons without exposing paths or remote errors.
const reasons: [RegExp, string][] = [
  [/FILE_TOO_BIG|too large|2 GB|TelegramLimit/i, 'action_errors.file_limit'],
  [/FLOOD_WAIT|Telegram cooling down/i, 'activity.category_rate_limit'],
  [/passphrase required|credentials required|VAULT_LOCKED|KEY_REQUIRED/i, 'activity.category_unlock'],
  [/save this (upload|download) for background recovery/i, 'activity.category_persistence'],
  [/Wi-Fi/i, 'offlinePacks.waiting_wifi'],
  [/waiting for .*network/i, 'offlinePacks.waiting_network'],
  [/no space|disk full|storage is low|free storage|permission denied/i, 'activity.category_storage'],
  [/network|connection|timed out/i, 'activity.category_network'],
  [/not found|no such file/i, 'activity.category_source_missing'],
  [/integrity|verification failed/i, 'activity.category_integrity'],
  [/ACCOUNT_/, 'activity.category_account'],
];

export function transferError({ error, errorCategory }: Failure, t: Translate): string {
  const reason = reasons.find(([pattern]) => pattern.test(error ?? ''))?.[1];
  if (errorCategory && /^(network|rate_limit|unlock|source_missing|storage|integrity|account|interrupted|persistence|cancelled)$/.test(errorCategory)) return t('activity.category_' + errorCategory);
  return t(reason ?? 'activity.category_other');
}
