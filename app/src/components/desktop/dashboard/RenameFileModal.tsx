import { useState, useRef, useEffect, useId } from 'react';
import { Pencil, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useModalFocus } from '../../../hooks/useModalFocus';

interface RenameFileModalProps {
    fileName: string;
    onRename: (newName: string) => Promise<void>;
    onClose: () => void;
}

export function RenameFileModal({ fileName, onRename, onClose }: RenameFileModalProps) {
    const [name, setName] = useState(fileName);
    const [isSubmitting, setIsSubmitting] = useState(false);
    const inputRef = useRef<HTMLInputElement>(null);
    const panelRef = useRef<HTMLDivElement>(null);
    const titleId = useId();
    const { t } = useTranslation();
    useModalFocus(panelRef, onClose);

    useEffect(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
    }, []);

    const handleSubmit = async () => {
        if (isSubmitting) return;
        const trimmed = name.trim();
        if (!trimmed || trimmed === fileName) {
            onClose();
            return;
        }
        setIsSubmitting(true);
        try {
            await onRename(trimmed);
            onClose();
        } catch {
            setIsSubmitting(false);
        }
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleSubmit();
        } else if (e.key === 'Escape') {
            onClose();
        }
    };

    return (
        <div
            className="fixed inset-0 z-[250] flex items-center justify-center bg-black/50 backdrop-blur-sm"
            onClick={onClose}
        >
            <div
                ref={panelRef}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                tabIndex={-1}
                className="bg-telegram-surface border border-telegram-border rounded-xl w-[360px] shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-150"
                onClick={e => e.stopPropagation()}
            >
                <div className="p-4 border-b border-telegram-border flex items-center justify-between">
                    <h3 id={titleId} className="text-telegram-text font-medium flex items-center gap-2">
                        <Pencil className="w-4 h-4 text-blue-400" />
                        {t('files.rename_file')}
                    </h3>
                    <button
                        aria-label={t('common.close')}
                        onClick={onClose}
                        className="text-telegram-subtext hover:text-telegram-text transition-colors"
                        disabled={isSubmitting}
                    >
                        <X className="w-4 h-4" />
                    </button>
                </div>

                <div className="p-4 space-y-3">
                    <input
                        ref={inputRef}
                        type="text"
                        aria-label={t('files.file_name')}
                        value={name}
                        onChange={e => setName(e.target.value)}
                        onKeyDown={handleKeyDown}
                        maxLength={200}
                        className="w-full bg-telegram-bg border border-telegram-border rounded-lg px-3 py-2 text-sm text-telegram-text placeholder:text-telegram-subtext/50 focus:outline-none focus:ring-2 focus:ring-telegram-primary/50 focus:border-telegram-primary/50 transition-all"
                        placeholder={t('files.file_name')}
                        disabled={isSubmitting}
                    />
                </div>

                <div className="p-4 border-t border-telegram-border flex justify-end gap-2 bg-telegram-hover/10">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 text-sm font-medium text-telegram-subtext hover:text-telegram-text bg-telegram-hover/50 hover:bg-telegram-hover rounded-lg transition-colors"
                        disabled={isSubmitting}
                    >
                        {t('common.cancel')}
                    </button>
                    <button
                        onClick={handleSubmit}
                        disabled={isSubmitting || !name.trim() || name.trim() === fileName}
                        className="px-4 py-2 text-sm font-medium text-white bg-telegram-primary hover:bg-telegram-primary/90 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg transition-colors"
                    >
                        {isSubmitting ? t('files.renaming') : t('files.rename')}
                    </button>
                </div>
            </div>
        </div>
    );
}
