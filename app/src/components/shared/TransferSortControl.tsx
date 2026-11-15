import { useTranslation } from 'react-i18next';
import type { TransferSortOrder } from '../../services/transferSort';

export function TransferSortControl({ value, onChange, touch = false }: {
  value: TransferSortOrder;
  onChange: (value: TransferSortOrder) => void;
  touch?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <label className="flex min-w-0 max-w-full flex-wrap items-center gap-2 text-metadata text-app-text-secondary">
      <span>{t('activity.sort_transfers')}</span>
      <select
        value={value}
        aria-label={t('activity.sort_transfers')}
        onChange={event => onChange(event.target.value === 'newest' ? 'newest' : 'oldest')}
        className={`app-select quiet-control h-8 min-w-0 max-w-full border border-app-border bg-app-surface-sunken/45 px-2.5 text-ui text-app-text outline-none focus:border-app-accent/60 ${touch ? 'min-h-11' : ''}`}
      >
        <option value="oldest">{t('activity.oldest_first')}</option>
        <option value="newest">{t('activity.newest_first')}</option>
      </select>
    </label>
  );
}
