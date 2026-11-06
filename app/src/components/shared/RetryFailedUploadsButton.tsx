import { RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/Button';

export function RetryFailedUploadsButton({ count, busy, onRetry, touch = false }: {
  count: number;
  busy: boolean;
  onRetry: () => void;
  touch?: boolean;
}) {
  const { t } = useTranslation();
  if (!count && !busy) return null;
  return <Button size="sm" disabled={busy || !count} aria-busy={busy} onClick={onRetry}
    className={touch ? 'min-h-11 max-w-full' : 'max-w-full'}
    leadingIcon={<RotateCcw className={`h-3.5 w-3.5 shrink-0 ${busy ? 'animate-spin' : ''}`} aria-hidden="true" />}>
    {t('activity.retry_all_failed')} ({count})
  </Button>;
}
