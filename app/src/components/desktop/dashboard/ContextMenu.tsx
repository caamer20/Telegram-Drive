import i18n from '../../../i18n';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Eye, HardDrive, Trash2, FolderOpen, Pencil, Play, FileText, Link, Copy, ArrowRightLeft, Star, Pin, Info, FolderInput } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { TelegramFile, TelegramFolder } from '../../../types';
import { toast } from 'sonner';
import { describeFileActions, resolvePublicFolderUsername } from './fileActionDescriptors';
import { requestFileOrganization } from '../../../services/workspace';

interface ContextMenuProps {
    x: number;
    y: number;
    file: TelegramFile;
    onClose: () => void;
    onDownload: () => void;
    onDelete: () => void;
    onPreview: () => void;
    onShare?: () => void;
    onRename?: () => void;
    onMove?: () => void;
    onCopy?: () => void;
    onProperties?: () => void;
    folders?: TelegramFolder[];
    activeFolderId?: number | null;
    onToggleFavorite?: () => void;
    onTogglePinned?: () => void;
}

export function ContextMenu({ x, y, file, onClose, onDownload, onDelete, onPreview, onShare, onRename, onMove, onCopy, onProperties, folders, activeFolderId, onToggleFavorite, onTogglePinned }: ContextMenuProps) {
    const [adjustedPos, setAdjustedPos] = useState({ x, y });
    const menuRef = useRef<HTMLDivElement>(null);
    const { t } = useTranslation();
    const actions = describeFileActions(file);

    // Adjust position to stay in bounds
    useLayoutEffect(() => {
        if (menuRef.current) {
            const rect = menuRef.current.getBoundingClientRect();
            let newX = x;
            let newY = y;

            if (x + rect.width > window.innerWidth) {
                newX = x - rect.width;
            }
            if (y + rect.height > window.innerHeight) {
                newY = y - rect.height;
            }
            setAdjustedPos({ x: newX, y: newY });
        }
    }, [x, y]);

    // Close on outside click — ignore clicks inside the menu so button handlers can fire
    useEffect(() => {
        const handleClick = (e: MouseEvent) => {
            if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
                onClose();
            }
        };
        const handleResize = () => onClose();
        const handleContextMenu = (e: MouseEvent) => {
            if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
                onClose();
            }
        };

        window.addEventListener('click', handleClick, true);
        window.addEventListener('resize', handleResize);
        window.addEventListener('contextmenu', handleContextMenu, true);

        return () => {
            window.removeEventListener('click', handleClick, true);
            window.removeEventListener('resize', handleResize);
            window.removeEventListener('contextmenu', handleContextMenu, true);
        };
    }, [onClose]);

    return (
        <div
            ref={menuRef}
            className="quiet-menu fixed z-50 flex min-w-[220px] flex-col gap-0.5 p-1.5 animate-in fade-in zoom-in-95 duration-100"
            style={{ left: adjustedPos.x, top: adjustedPos.y }}
            onClick={(e) => e.stopPropagation()}
            onContextMenu={(e) => e.preventDefault()}
        >
            <div className="px-2 py-1.5 text-xs text-telegram-subtext font-medium truncate max-w-[180px] border-b border-telegram-border mb-1">
                {file.name}
            </div>

            {!actions.isFolder && (
                <button onClick={onPreview} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    {actions.previewAction === 'play' ? (
                        <>
                            <Play className="w-4 h-4 text-telegram-primary" />
                            {t('common.play')}
                        </>
                    ) : actions.previewAction === 'view_pdf' ? (
                        <>
                            <FileText className="w-4 h-4 text-red-400" />
                            {t('files.view_pdf')}
                        </>
                    ) : (
                        <>
                            <Eye className="w-4 h-4 text-blue-500" />
                            {t('files.preview')}
                        </>
                    )}
                </button>
            )}

            {actions.isFolder && (
                <button onClick={onPreview} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <FolderOpen className="w-4 h-4 text-yellow-500" />
                    {t('files.open')}
                </button>
            )}

            <button onClick={onDownload} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                <HardDrive className="w-4 h-4 text-green-500" />
                {t('files.download')}
            </button>

            {!actions.isFolder && onToggleFavorite && (
                <button onClick={onToggleFavorite} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-telegram-text transition-colors hover:bg-telegram-hover">
                    <Star className={`h-4 w-4 ${file.is_favorite ? 'fill-amber-400 text-amber-400' : 'text-amber-400'}`} />
                    {file.is_favorite ? i18n.t('navigation_copy.favorite_remove') : i18n.t('navigation_copy.favorite_add')}
                </button>
            )}
            {!actions.isFolder && <button type="button" onClick={() => { requestFileOrganization(file, activeFolderId ?? null); onClose(); }} className="flex min-h-9 w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-telegram-text hover:bg-telegram-hover"><FolderOpen className="h-4 w-4 text-telegram-primary" />{t('workspace.organize_file')}</button>}
            {!actions.isFolder && onTogglePinned && (
                <button onClick={onTogglePinned} className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-telegram-text transition-colors hover:bg-telegram-hover">
                    <Pin className={`h-4 w-4 ${file.is_pinned ? 'fill-blue-400 text-blue-400' : 'text-blue-400'}`} />
                    {file.is_pinned ? i18n.t('navigation_copy.unpin') : i18n.t('navigation_copy.pin')}
                </button>
            )}

            {actions.canShare && onShare && (
                <button onClick={onShare} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <Link className="w-4 h-4 text-telegram-primary" />
                    {t('files.share_link')}
                </button>
            )}

            {!actions.isFolder && (
                (() => {
                    const username = resolvePublicFolderUsername(file, folders, activeFolderId);
                    
                    if (username) {
                        const handleCopyLink = async () => {
                            const url = `https://t.me/${username}/${file.id}`;
                            try {
                                await navigator.clipboard.writeText(url);
                                toast.success(t('notifications.telegram_link_copied'));
                            } catch (e) {
                                toast.error(t('notifications.copy_link_failed'));
                            }
                            onClose();
                        };
                        return (
                            <button onClick={handleCopyLink} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                                <Copy className="w-4 h-4 text-telegram-primary" />
                                {t('files.copy_telegram_link')}
                            </button>
                        );
                    } else {
                        return (
                            <button 
                                disabled 
                                title={i18n.t('navigation_copy.public_only')}
                                className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-subtext hover:bg-telegram-hover rounded transition-colors text-left w-full cursor-not-allowed opacity-50"
                            >
                                <Copy className="w-4 h-4" />
                                {t('files.copy_telegram_link')}
                            </button>
                        );
                    }
                })()
            )}

            {actions.canMove && onMove && (
                <button onClick={onMove} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <ArrowRightLeft className="w-4 h-4 text-amber-400" />
                    {t('files.move_to_folder')}
                </button>
            )}

            {actions.canMove && onCopy && (
                <button onClick={onCopy} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <FolderInput className="w-4 h-4 text-teal-400" />
                    Copy to Folder
                </button>
            )}

            {actions.canRename && onRename && (
                <button onClick={onRename} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <Pencil className="w-4 h-4 text-blue-400" />
                    {t('files.rename')}
                </button>
            )}

            {actions.canRename && !onRename && (
                <button disabled className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-subtext hover:bg-telegram-hover rounded transition-colors text-left w-full cursor-not-allowed opacity-50">
                    <Pencil className="w-4 h-4" />
                    {t('files.rename')}
                </button>
            )}

            <div className="h-px bg-telegram-border my-1" />

            {onProperties && (
                <button onClick={onProperties} className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full">
                    <Info className="w-4 h-4 text-blue-400" />
                    Properties
                </button>
            )}

            <button onClick={onDelete} className="flex items-center gap-2 px-2 py-1.5 text-sm text-red-500 hover:bg-red-500/10 rounded transition-colors text-left w-full">
                <Trash2 className="w-4 h-4" />
                {t('files.delete')}
            </button>
        </div>
    );
}
