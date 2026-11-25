import { Trans, useTranslation } from 'react-i18next';

const tag = '[TD]';

export default function ExistingChannelHelp() {
    const { t } = useTranslation();
    const before = t('existing_channel_help.example');

    return (
        <details className="text-xs text-app-text-secondary">
            <summary className="min-h-11 cursor-pointer rounded-lg px-2 py-2.5 font-medium leading-5 text-app-accent">
                {t('existing_channel_help.title')}
            </summary>
            <ol className="list-decimal space-y-2 select-text ps-5 pb-3 pe-2 leading-5">
                <li>{t('existing_channel_help.edit')}</li>
                <li>
                    <Trans
                        i18nKey="existing_channel_help.rename"
                        values={{ tag, before, after: `${before} ${tag}` }}
                        components={{ tag: <bdi dir="ltr" />, before: <bdi />, after: <bdi dir="ltr" /> }}
                    />
                </li>
                <li>{t('existing_channel_help.sync', { sync: t('common.sync') })}</li>
            </ol>
        </details>
    );
}
