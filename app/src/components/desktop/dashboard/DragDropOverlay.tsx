import { UploadCloud } from 'lucide-react';
import { useTranslation } from 'react-i18next';

export function DragDropOverlay({ currentFolderName, fileCount }: { currentFolderName: string; fileCount: number }) {
    const { t } = useTranslation();

    return (
        <div
            data-ui-motion
            className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center pointer-events-none"
        >
            <div
                data-ui-motion
                className="ui-enter glass bg-telegram-surface border border-telegram-primary/50 text-telegram-text rounded-2xl p-8 flex flex-col items-center gap-4 shadow-2xl"
            >
                <div className="p-4 bg-telegram-primary/10 rounded-full">
                    <UploadCloud className="w-12 h-12 text-telegram-primary animate-bounce" />
                </div>
                <div className="text-center">
                    <h3 className="text-xl font-bold text-telegram-text">
                        {t('files.drop_to_upload', { folder: currentFolderName })}
                    </h3>
                    <p className="mt-1 text-sm text-telegram-subtext">
                        {t('files.drop_hint', { count: fileCount })}
                    </p>
                </div>
            </div>
        </div>
    );
}
