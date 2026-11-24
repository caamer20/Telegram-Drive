import { useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';

interface Receipt { id: string; code: string }

export function SupporterRecoveryReceipt({ storageHelp, deliveryPending }: { storageHelp: string; deliveryPending: boolean }) {
  const { t } = useTranslation();
  const title = useId();
  const details = useId();
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [failed, setFailed] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let open = true;
    setFailed(false);
    void invoke<Receipt | null>('cmd_pending_supporter_receipt').then(value => {
      if (open) setReceipt(value);
    }).catch(() => { if (open) setFailed(true); });
    return () => { open = false; };
  }, [attempt, deliveryPending]);

  const confirm = async () => {
    if (!receipt || saving || deliveryPending) return;
    setSaving(true);
    try {
      await invoke('cmd_acknowledge_supporter_receipt', { receiptId: receipt.id });
      setReceipt(null);
      setFailed(false);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  if (!failed && !receipt) return null;
  return (
    <div role="group" aria-labelledby={title} className="mt-3 rounded-lg border border-current/25 p-3 text-xs leading-5">
      <strong id={title}>{t('supporter_license.save_recovery')}</strong>
      {receipt && <>
        <code dir="ltr" className="mt-2 block select-all break-all font-mono">{receipt.code}</code>
        <p id={details} className="mt-2">{storageHelp}</p>
        {deliveryPending && <p role="status">{t('supporter_license.recovery_pending')}</p>}
        <button type="button" className="quiet-control mt-2 rounded-lg border px-3 py-2" aria-describedby={details} disabled={saving || deliveryPending} onClick={() => void confirm()}>{t('common.confirm')}</button>
      </>}
      {failed && <p role="alert">{t('supporter_license.error_storage')}</p>}
      {failed && !receipt && <button type="button" className="quiet-control rounded-lg border px-3 py-2" onClick={() => setAttempt(previous => previous + 1)}>{t('common.retry')}</button>}
    </div>
  );
}
