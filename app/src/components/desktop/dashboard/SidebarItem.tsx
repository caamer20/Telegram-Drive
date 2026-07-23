import { useState, useEffect, useRef, useCallback } from 'react';
import { MoreVertical, Globe, Pencil, Trash2, EyeOff, Eye, Link } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';

import { FolderGroup } from '../../../types';

interface SidebarItemProps {
    icon: React.ElementType;
    label: string;
    active: boolean;
    onClick: () => void;
    onDrop?: (e: React.DragEvent) => void;
    onDelete?: () => void;
    folderId: number | null;
    isPublic?: boolean;
    onRename?: () => void;
    onToggleVisibility?: () => void;
    onExportInvite?: () => void;
    collapsed?: boolean;
    groups?: FolderGroup[];
    onAssignFolderToGroup?: (folderId: number, groupId: number | null) => void;
}

/**
 * SidebarItem - Pure DOM event-based drop handling
 * 
 * With Tauri's dragDropEnabled: false, DOM events work reliably.
 * This component handles internal file moves via standard React drag events.
 * Right-click shows a context menu for folder management.
 */
export function SidebarItem({
    icon: Icon, label, active = false, onClick, onDrop, onDelete, folderId, isPublic, onRename, onToggleVisibility, onExportInvite, collapsed = false,
    groups = [], onAssignFolderToGroup
}: SidebarItemProps) {
    const [isOver, setIsOver] = useState(false);
    const [dragCount, setDragCount] = useState(0);
    const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
    const menuRef = useRef<HTMLDivElement>(null);
    const settingsBtnRef = useRef<HTMLDivElement>(null);
    const { t } = useTranslation();

    const {
        attributes,
        listeners,
        setNodeRef,
        transform,
        transition,
        isDragging,
    } = useSortable({
        id: folderId !== null ? `folder-${folderId}` : 'saved-messages',
        disabled: folderId === null,
    });

    const style = folderId !== null ? {
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.4 : undefined,
    } : undefined;

    const hasFolderActions = onDelete && folderId !== null;

    // Open the settings popover positioned relative to the ⋮ button
    const openSettingsPopover = useCallback((e: React.MouseEvent) => {
        e.stopPropagation();
        if (!settingsBtnRef.current) return;
        const rect = settingsBtnRef.current.getBoundingClientRect();
        setContextMenu({ x: rect.left - 200, y: rect.bottom + 4 });
    }, []);

    // Open context menu at mouse position (right-click)
    const openContextMenu = useCallback((e: React.MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (hasFolderActions) {
            setContextMenu({ x: e.clientX, y: e.clientY });
        }
    }, [hasFolderActions]);

    // Close context menu on outside click
    useEffect(() => {
        if (!contextMenu) return;
        const handler = () => setContextMenu(null);
        window.addEventListener('click', handler);
        window.addEventListener('contextmenu', handler);
        return () => {
            window.removeEventListener('click', handler);
            window.removeEventListener('contextmenu', handler);
        };
    }, [contextMenu]);

    // Adjust menu position to stay in viewport
    useEffect(() => {
        if (!contextMenu || !menuRef.current) return;
        const rect = menuRef.current.getBoundingClientRect();
        let newX = contextMenu.x;
        let newY = contextMenu.y;
        if (newX + rect.width > window.innerWidth) newX = newX - rect.width;
        if (newY + rect.height > window.innerHeight) newY = newY - rect.height;
        if (newX !== contextMenu.x || newY !== contextMenu.y) {
            setContextMenu({ x: newX, y: newY });
        }
    }, [contextMenu]);

    // Parse drop count from drag data so we can show a badge
    const parseDragCount = useCallback((e: React.DragEvent): number => {
        const rawIds = e.dataTransfer.getData("application/x-telegram-file-ids");
        if (rawIds) {
            try { const ids = JSON.parse(rawIds); if (Array.isArray(ids) && ids.length > 0) return ids.length; } catch { /* ignore */ }
        }
        const singleId = e.dataTransfer.getData("application/x-telegram-file-id");
        if (singleId) return 1;
        return 0;
    }, []);

    return (
        <div
            ref={setNodeRef}
            style={style}
            {...attributes}
            {...listeners}
            onClick={onClick}
            title={collapsed ? label : undefined}
            onDragEnter={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setIsOver(true);
                setDragCount(parseDragCount(e));
            }}
            onDragOver={(e) => {
                e.preventDefault();
                e.stopPropagation();
                e.dataTransfer.dropEffect = 'move';
            }}
            onDragLeave={(e) => {
                e.preventDefault();
                e.stopPropagation();
                const rect = e.currentTarget.getBoundingClientRect();
                const x = e.clientX;
                const y = e.clientY;
                if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) {
                    setIsOver(false);
                    setDragCount(0);
                }
            }}
            onDrop={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setIsOver(false);
                setDragCount(0);
                if (onDrop) onDrop(e);
            }}
            onContextMenu={openContextMenu}
            className={`group w-full flex items-center transition-all duration-150 cursor-pointer select-none ${collapsed ? 'justify-center p-2.5' : 'gap-3 px-3 py-2'} ${active
                ? 'bg-telegram-primary/10 text-telegram-primary'
                : isOver
                    ? 'bg-telegram-primary/30 text-telegram-text ring-2 ring-telegram-primary scale-[1.02] shadow-lg'
                    : 'text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text'
                }`}
        >
            <Icon className={`w-4 h-4 flex-shrink-0 ${isOver ? 'text-telegram-primary' : ''}`} />
            {!collapsed && <span className="flex-1 text-left truncate">{label}</span>}
            {isOver && dragCount > 1 && (
                <span className="flex-shrink-0 px-1.5 py-0.5 bg-telegram-primary text-white text-[10px] font-bold rounded-full leading-none min-w-[18px] text-center">
                    {dragCount}
                </span>
            )}
            {isPublic && !collapsed && (
                <Globe className="w-3 h-3 text-emerald-400 flex-shrink-0" />
            )}
            {onDelete && !collapsed && (
                <div
                    ref={settingsBtnRef}
                    onClick={openSettingsPopover}
                    className="opacity-0 group-hover:opacity-100 p-1 rounded hover:bg-telegram-hover transition-all"
                    title={t('files.folder_settings')}
                >
                    <MoreVertical className="w-3.5 h-3.5 text-telegram-subtext hover:text-telegram-text" />
                </div>
            )}

            {/* Folder Context Menu */}
            {contextMenu && (
                <div
                    ref={menuRef}
                    className="fixed z-[300] min-w-[200px] bg-telegram-surface/95 backdrop-blur-xl border border-telegram-border rounded-lg shadow-2xl p-1.5 flex flex-col gap-0.5 animate-in fade-in zoom-in-95 duration-100"
                    style={{ left: contextMenu.x, top: contextMenu.y }}
                    onClick={(e) => e.stopPropagation()}
                    onContextMenu={(e) => e.preventDefault()}
                >
                    <div className="px-2 py-1.5 text-xs text-telegram-subtext font-medium truncate max-w-[180px] border-b border-telegram-border mb-1">
                        {label}
                    </div>

                    {onRename && (
                        <button
                            onClick={() => { setContextMenu(null); onRename(); }}
                            className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full"
                        >
                            <Pencil className="w-4 h-4 text-blue-400" />
                            {t('files.rename')}
                        </button>
                    )}

                    {onToggleVisibility && (
                        <button
                            onClick={() => { setContextMenu(null); onToggleVisibility(); }}
                            className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full"
                        >
                            {isPublic ? (
                                <>
                                    <EyeOff className="w-4 h-4 text-amber-400" />
                                    {t('files.make_private')}
                                </>
                            ) : (
                                <>
                                    <Eye className="w-4 h-4 text-emerald-400" />
                                    {t('files.make_public')}
                                </>
                            )}
                        </button>
                    )}

                    {onExportInvite && (
                        <button
                            onClick={() => { setContextMenu(null); onExportInvite(); }}
                            className="flex items-center gap-2 px-2 py-1.5 text-sm text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full"
                        >
                            <Link className="w-4 h-4 text-telegram-primary" />
                            {t('files.copy_link')}
                        </button>
                    )}

                    {onAssignFolderToGroup && folderId !== null && groups && groups.length > 0 && (
                        <>
                            <div className="h-px bg-telegram-border my-1" />
                            <div className="px-2 py-1 text-[10px] font-semibold text-telegram-subtext uppercase tracking-wider">
                                {t('files.move_to_group') || "Move to Group"}
                            </div>
                            <button
                                onClick={() => { setContextMenu(null); onAssignFolderToGroup(folderId, null); }}
                                className="flex items-center gap-2 px-3 py-1.5 text-xs text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full"
                            >
                                <span className="w-1.5 h-1.5 rounded-full bg-telegram-subtext" />
                                {t('common.unassigned') || "None (Unassigned)"}
                            </button>
                            {groups.map(group => (
                                <button
                                    key={group.id}
                                    onClick={() => { setContextMenu(null); onAssignFolderToGroup(folderId, group.id); }}
                                    className="flex items-center gap-2 px-3 py-1.5 text-xs text-telegram-text hover:bg-telegram-hover rounded transition-colors text-left w-full"
                                >
                                    <span className="w-1.5 h-1.5 rounded-full" style={{ backgroundColor: group.color_hex }} />
                                    {group.name}
                                </button>
                            ))}
                        </>
                    )}

                    <div className="h-px bg-telegram-border my-1" />

                    <button
                        onClick={() => { setContextMenu(null); onDelete?.(); }}
                        className="flex items-center gap-2 px-2 py-1.5 text-sm text-red-500 hover:bg-red-500/10 rounded transition-colors text-left w-full"
                    >
                        <Trash2 className="w-4 h-4" />
                        {t('files.delete')}
                    </button>
                </div>
            )}
        </div>
    )
}
