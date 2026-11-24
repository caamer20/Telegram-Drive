import { SupporterRecoveryReceipt } from '../../../shared/SupporterRecoveryReceipt';
import { useEffect, useState } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { AlertTriangle, CheckCircle2, CreditCard, Heart, KeyRound, Megaphone } from 'lucide-react';
import { open } from '@tauri-apps/plugin-shell';
import { toast } from 'sonner';
import { useSupporter } from '../../../../context/SupporterContext';
import { shouldOfferNewSupporterPurchase } from '../../../../services/supporterVisibility';
import { supporterErrorMessage, supporterPaymentMessage, supporterStatusMessage } from '../../../../services/supporterMessages';

export function SupporterSettingsSection() {
  const { t } = useTranslation();
  const { status, beginCheckout, pollCheckout, activate, refreshEntitlement } = useSupporter();
  const [acceptedTerms, setAcceptedTerms] = useState(false);
  const [checkoutPending, setCheckoutPending] = useState(false);
  const [busy, setBusy] = useState(false);
  const [recoveryCode, setRecoveryCode] = useState('');
  const termsUrl = status.terms_url ?? 'https://github.com/caamer20/Telegram-Drive/blob/main/SUPPORTER_TERMS.md';
  const supportUrl = 'https://github.com/caamer20/Telegram-Drive/issues/new/choose';
  const canPurchase = shouldOfferNewSupporterPurchase(status) && !status.checkout_pending;
  const canRefresh = ['active', 'needs_refresh', 'expired'].includes(status.state);
  const canRestore = !status.ad_free && !['loading', 'unavailable', 'revoked'].includes(status.state);
  const isReturningSupporter = !status.ad_free && !canPurchase
    && (status.state === 'expired' || status.recovery_code_saved);
  const statusTitle = status.state === 'loading'
    ? t('supporter_license.checking')
    : status.state === 'revoked'
      ? t('supporter_license.revoked')
      : status.state === 'expired'
        ? t('supporter_license.expired')
        : status.state === 'unavailable'
          ? t('supporter_license.unavailable')
          : status.recovery_code_saved
            ? t('supporter_license.previous_purchase')
            : t('supporter_license.inactive');

  const checkPayment = async (quiet = false) => {
    try {
      const result = await pollCheckout();
      if (result.status === 'completed') {
        toast.success(supporterPaymentMessage(result, t));
      } else if (!quiet) {
        toast.info(supporterPaymentMessage(result, t));
      }
    } catch (error) {
      if (!quiet) toast.error(supporterErrorMessage(error, t));
    }
  };

  useEffect(() => {
    setCheckoutPending(Boolean(status.checkout_pending));
  }, [status.ad_free, status.checkout_pending]);

  const startCheckout = async () => {
    setBusy(true);
    try {
      const checkout = await beginCheckout(status.terms_version);
      await open(checkout.approval_url);
      setCheckoutPending(true);
    } catch (error) {
      toast.error(supporterErrorMessage(error, t));
    } finally {
      setBusy(false);
    }
  };

  const recoverPurchase = async () => {
    if (!recoveryCode.trim()) return;
    setBusy(true);
    try {
      await activate(recoveryCode.trim(), status.terms_version);
      setRecoveryCode('');
      toast.success(t('supporter_license.purchase_restored'));
    } catch (error) {
      toast.error(supporterErrorMessage(error, t));
    } finally {
      setBusy(false);
    }
  };

  const recoveryNotice = (status.ad_free || status.recovery_code_saved)
    ? <SupporterRecoveryReceipt deliveryPending={Boolean(status.checkout_pending)} storageHelp={t('supporter_license.recovery_storage_desktop')} />
    : null;

  if (status.ad_free) {
    return (
      <section id="desktop-supporter-section" tabIndex={-1} className="rounded-lg border border-app-success/25 bg-app-success/5 p-4" aria-labelledby="supporter-settings-title">
        <div className="flex items-center gap-3">
          <CheckCircle2 className="h-5 w-5 shrink-0 text-app-success" aria-hidden="true" />
          <h3 id="supporter-settings-title" className="text-sm font-semibold text-app-text">{t('supporter_license.purchased_title')}</h3>
        </div>
        {recoveryNotice}
        <details className="mt-4 text-xs leading-5 text-app-text-secondary">
          <summary className="cursor-pointer font-medium text-app-text">{t('supporter_license.manage_license')}</summary>
          <p className="mt-3">{supporterStatusMessage(status, t)}</p>
          <p className="mt-2">{t('supporter_license.updates_preserved')}</p>
          <p className="mt-3"><strong className="text-app-text">{t('supporter_license.step_recovery_title')}</strong>{' '}{t('supporter_license.recovery_storage_desktop')}</p>
          <div className="mt-3 flex flex-wrap items-center gap-3">
            {checkoutPending && <button type="button" onClick={() => void checkPayment()} className="quiet-control px-4 py-2.5 text-xs font-medium text-app-text">{t('supporter_license.check_payment')}</button>}
            {canRefresh && <button type="button" onClick={() => void refreshEntitlement().then(() => toast.success(t('supporter_license.verification_refreshed'))).catch(error => toast.error(supporterErrorMessage(error, t)))} className="quiet-control px-4 py-2.5 text-xs font-medium text-app-text">{t('supporter_license.refresh_verification')}</button>}
            <button type="button" onClick={() => void open(termsUrl)} className="quiet-control px-3 py-2.5 text-xs text-app-text-secondary">{t('supporter_license.terms_action')}</button>
            <button type="button" onClick={() => void open(supportUrl)} className="quiet-control px-3 py-2.5 text-xs text-app-text-secondary">{t('supporter_license.help_action')}</button>
          </div>
        </details>
      </section>
    );
  }

  return (
    <section id="desktop-supporter-section" tabIndex={-1} className="rounded-lg border border-app-accent/20 bg-app-accent/5 p-4" aria-labelledby="supporter-settings-title">
      <div className="flex items-start gap-3">
        <Heart className="mt-0.5 h-5 w-5 shrink-0 text-app-accent" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h3 id="supporter-settings-title" className="text-sm font-semibold text-app-text">{t('supporter_license.nav_title')}</h3>
          <p className="mt-1 text-xs leading-5 text-app-text-secondary">{t('supporter_license.description')}</p>
          <div className={`mt-3 rounded-lg border p-3 text-xs leading-5 text-app-text-secondary ${status.state === 'revoked' ? 'border-app-danger/25 bg-app-danger/5' : 'border-app-border-subtle bg-app-surface-sunken/25'}`}>
            <strong className={status.state === 'revoked' ? 'text-app-danger' : 'text-app-text'}>{statusTitle}</strong>
            <span className="mt-1 block">{supporterStatusMessage(status, t)}</span>
            {isReturningSupporter && <span className="mt-1 block font-medium text-app-text">{t('supporter_license.returning_warning')}</span>}
          </div>
        </div>
      </div>

      {canPurchase && (
        <div className="mt-5 space-y-4">
          <div className="rounded-xl border border-app-accent/30 bg-app-surface p-5 text-center shadow-sm">
            <span className="text-xs font-semibold uppercase tracking-[0.16em] text-app-accent">{t('supporter_license.ad_free_life')}</span>
            <div className="mt-2 text-3xl font-semibold tracking-tight text-app-text">$5 <span className="text-sm font-medium text-app-text-secondary">{t('supporter_offer.price_suffix')}</span></div>
            <p className="mt-2 text-sm font-medium text-app-text">{t('supporter_license.single_payment')}</p>
            <p className="mt-1 text-xs leading-5 text-app-text-secondary">{t('supporter_license.devices_updates')}</p>
          </div>

          <div className="grid gap-3 sm:grid-cols-2" aria-label={t('supporter_license.comparison_label')}>
            <div className="rounded-lg border border-app-border-subtle bg-app-surface-sunken/20 p-4">
              <strong className="text-sm text-app-text">{t('supporter_license.free_forever')}</strong>
              <ul className="mt-3 space-y-2 text-xs leading-5 text-app-text-secondary">
                <li className="flex gap-2"><CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />{t('supporter_license.all_features')}</li>
                <li className="flex gap-2"><CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />{t('supporter_license.no_account_subscription')}</li>
                <li className="flex gap-2"><Megaphone className="mt-0.5 h-4 w-4 shrink-0 text-app-text-tertiary" />{t('supporter_license.sponsors_labeled')}</li>
              </ul>
            </div>
            <div className="rounded-lg border border-app-accent/30 bg-app-accent/5 p-4">
              <strong className="text-sm text-app-text">{t('supporter_license.supporter_tier')}</strong>
              <ul className="mt-3 space-y-2 text-xs leading-5 text-app-text-secondary">
                <li className="flex gap-2"><CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />{t('supporter_license.all_features')}</li>
                <li className="flex gap-2"><CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />{t('supporter_license.sponsors_removed')}</li>
                <li className="flex gap-2"><CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />{t('supporter_license.updates_activated')}</li>
              </ul>
            </div>
          </div>

          <div>
            <h4 className="text-xs font-semibold uppercase tracking-wider text-app-text-secondary">{t('supporter_license.activation_heading')}</h4>
            <ol className="mt-3 grid gap-3 sm:grid-cols-3">
              {([
                [CreditCard, '1', t('supporter_license.step_pay_title'), t('supporter_license.step_pay_description')],
                [CheckCircle2, '2', t('supporter_license.step_return_title'), t('supporter_license.step_return_description')],
                [KeyRound, '3', t('supporter_license.step_recovery_title'), t('supporter_license.step_recovery_description')],
              ] as const).map(([Icon, number, title, description]) => (
                <li key={number} className="rounded-lg border border-app-border-subtle bg-app-surface-sunken/20 p-3">
                  <div className="flex items-center gap-2"><span className="flex h-6 w-6 items-center justify-center rounded-full bg-app-selected text-xs font-semibold text-app-accent">{number}</span><Icon className="h-4 w-4 text-app-accent" /></div>
                  <strong className="mt-2 block text-xs text-app-text">{title}</strong>
                  <p className="mt-1 text-xs leading-5 text-app-text-secondary">{description}</p>
                </li>
              ))}
            </ol>
          </div>

          <p className="rounded-lg border border-app-success/20 bg-app-success/5 p-3 text-xs leading-5 text-app-text-secondary"><strong className="text-app-text">{t('supporter_license.privacy_heading')}</strong>{' '}{t('supporter_license.privacy_description')}</p>
        </div>
      )}

      {recoveryNotice}

      {checkoutPending && (
        <div role="status" className="mt-4 rounded-lg border border-app-accent/25 bg-app-accent/5 p-3 text-xs leading-5 text-app-text-secondary">
          <strong className="text-app-text">{t('supporter_license.waiting_payment')}</strong>
          <p className="mt-1">{t('supporter_license.waiting_payment_desktop')}</p>
        </div>
      )}

      {canPurchase && (
        <details className="mt-4 rounded-lg border border-app-warning/25 bg-app-warning/5 p-3 text-xs leading-5 text-app-text-secondary">
          <summary className="cursor-pointer font-medium text-app-text"><span className="inline-flex items-center gap-2"><AlertTriangle className="h-4 w-4 text-app-warning" />{t('supporter_license.purchase_info')}</span></summary>
          <p className="mt-2">{t('supporter_license.purchase_policy_desktop')}</p>
        </details>
      )}

      {(canPurchase || canRestore) && (
        <label className="mt-4 flex cursor-pointer items-start gap-3 text-xs leading-5 text-app-text-secondary">
          <input type="checkbox" checked={acceptedTerms} onChange={event => setAcceptedTerms(event.target.checked)} className="mt-1 accent-[var(--color-app-accent)]" />
          <span><Trans t={t} i18nKey="supporter_license.consent_desktop" components={{ terms: <button type="button" onClick={event => { event.preventDefault(); void open(termsUrl); }} className="text-app-accent underline underline-offset-2" /> }} /></span>
        </label>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-3">
        {canPurchase && (
          <button type="button" disabled={!acceptedTerms || busy || checkoutPending} onClick={() => void startCheckout()} className="quiet-control bg-app-accent px-5 py-3 text-sm font-semibold text-app-accent-contrast disabled:cursor-not-allowed disabled:opacity-50">{busy ? t('supporter_license.preparing_checkout') : checkoutPending ? t('supporter_license.checkout_opened') : t('supporter_license.purchase_action')}</button>
        )}
        {checkoutPending && <button type="button" disabled={busy} onClick={() => void startCheckout()} className="quiet-control px-4 py-2.5 text-xs font-medium text-app-text">{t('supporter_offer.resume_checkout')}</button>}
        {checkoutPending && <button type="button" onClick={() => void checkPayment()} className="quiet-control px-4 py-2.5 text-xs font-medium text-app-text">{t('supporter_license.check_payment')}</button>}
        {canRefresh && <button type="button" onClick={() => void refreshEntitlement().then(() => toast.success(t('supporter_license.verification_refreshed'))).catch(error => toast.error(supporterErrorMessage(error, t)))} className="quiet-control px-4 py-2.5 text-xs font-medium text-app-text">{t('supporter_license.refresh_verification')}</button>}
        <button type="button" onClick={() => void open(termsUrl)} className="quiet-control px-3 py-2.5 text-xs text-app-text-secondary">{t('supporter_license.terms_action')}</button>
        <button type="button" onClick={() => void open(supportUrl)} className="quiet-control px-3 py-2.5 text-xs text-app-text-secondary">{t('supporter_license.help_action')}</button>
      </div>

      {canRestore && (
        <details className="mt-4 rounded-lg border border-app-border-subtle bg-app-surface-sunken/20 p-3">
          <summary className="cursor-pointer text-xs font-medium text-app-text">{t('supporter_license.restore_heading_desktop')}</summary>
          <p className="mt-2 text-xs leading-5 text-app-text-secondary">{t('supporter_license.restore_description_desktop')}</p>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row">
            <input dir="ltr" aria-label={t('supporter_license.recovery_code_label')} value={recoveryCode} onChange={event => setRecoveryCode(event.target.value)} placeholder={t('supporter_license.recovery_example')} autoComplete="off" spellCheck={false} className="min-w-0 flex-1 rounded-control border border-app-border bg-app-surface px-3 py-2 text-xs text-app-text outline-none focus:border-app-accent" />
            <button type="button" disabled={!acceptedTerms || !recoveryCode.trim() || busy} onClick={() => void recoverPurchase()} className="quiet-control px-4 py-2 text-xs font-medium text-app-text disabled:opacity-50">{t('supporter_license.restore_action_desktop')}</button>
          </div>
          <p className="mt-2 text-xs leading-5 text-app-text-tertiary">{t('supporter_license.restore_help')}</p>
        </details>
      )}
    </section>
  );
}
