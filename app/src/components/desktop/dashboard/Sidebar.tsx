import { useState } from 'react';
import { HardDrive, Folder, Plus, RefreshCw, LogOut, ChevronLeft, ChevronRight, Settings2, Trash2, Check, X, Eye, EyeOff, Cloud } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SidebarItem } from './SidebarItem';
import { BandwidthWidget } from './BandwidthWidget';
import { TelegramFolder, BandwidthStats, FolderGroup } from '../../../types';
import { useSettings } from '../../../context/SettingsContext';
import {
    DndContext,
    closestCenter,
    KeyboardSensor,
    PointerSensor,
    useSensor,
    useSensors,
    DragEndEvent,
} from '@dnd-kit/core';
import {
    arrayMove,
    SortableContext,
    sortableKeyboardCoordinates,
    verticalListSortingStrategy,
    horizontalListSortingStrategy,
    useSortable,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';

const PRESET_COLORS = [
    '#3B82F6', // Blue
    '#10B981', // Green
    '#8B5CF6', // Purple
    '#EC4899', // Pink
    '#F59E0B', // Orange
    '#14B8A6', // Teal
    '#06B6D4', // Cyan
    '#EF4444', // Red
];

interface GroupTabProps {
    id: string;
    groupId: number | null | 'all';
    label: string;
    colorHex?: string;
    active: boolean;
    onClick: () => void;
    onEdit?: () => void;
    isSortable?: boolean;
}

function GroupTab({ id, groupId, label, colorHex, active, onClick, onEdit, isSortable = true }: GroupTabProps) {
    const {
        attributes,
        listeners,
        setNodeRef,
        transform,
        transition,
        isDragging,
    } = useSortable({
        id,
        disabled: !isSortable,
    });

    const style = {
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.5 : undefined,
    };

    return (
        <div
            ref={setNodeRef}
            style={style}
            {...attributes}
            {...listeners}
            onClick={onClick}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-semibold select-none cursor-pointer transition-all duration-150 flex-shrink-0 border ${
                active
                    ? 'bg-telegram-primary/20 border-telegram-primary text-telegram-primary'
                    : 'bg-telegram-surface border-telegram-border text-telegram-subtext hover:text-telegram-text hover:border-telegram-primary/40'
            }`}
        >
            {colorHex && (
                <span
                    className="w-2 h-2 rounded-full flex-shrink-0"
                    style={{ backgroundColor: colorHex }}
                />
            )}
            <span className="truncate max-w-[80px]">{label}</span>
            {onEdit && active && groupId !== 'all' && groupId !== null && (
                <button
                    onClick={(e) => {
                        e.stopPropagation();
                        onEdit();
                    }}
                    className="p-0.5 hover:bg-telegram-hover rounded text-telegram-subtext hover:text-telegram-text transition-colors"
                >
                    <Settings2 className="w-3 h-3" />
                </button>
            )}
        </div>
    );
}

interface SidebarProps {
    folders: TelegramFolder[];
    groups: FolderGroup[];
    activeFolderId: number | null;
    setActiveFolderId: (id: number | null) => void;
    onDrop: (e: React.DragEvent, folderId: number | null) => void;
    onDelete: (id: number, name: string) => void;
    onRename: (id: number, name: string) => void;
    onToggleVisibility: (id: number, name: string, isPublic: boolean) => void;
    onExportInvite: (id: number, name: string) => void;
    onCreate: (name: string) => Promise<void>;
    isSyncing: boolean;
    isConnected: boolean;
    onSync: () => void;
    onLogout: () => void;
    bandwidth: BandwidthStats | null;
    onAssignFolderToGroup: (folderId: number, groupId: number | null) => Promise<void>;
    onReorderFolders: (reordered: TelegramFolder[]) => Promise<void>;
    onUpdateGroupOrder: (reorderedGroups: FolderGroup[]) => Promise<void>;
    onCreateGroup: (name: string, colorHex: string) => Promise<void>;
    onUpdateGroup: (groupId: number, name: string, colorHex: string) => Promise<void>;
    onDeleteGroup: (groupId: number) => Promise<void>;
    activeView?: 'files' | 'onedrive';
    onViewChange?: (view: 'files' | 'onedrive') => void;
}

export function Sidebar({
    folders, groups = [], activeFolderId, setActiveFolderId, onDrop, onDelete, onRename, onToggleVisibility, onExportInvite, onCreate,
    isSyncing, isConnected, onSync, onLogout, bandwidth,
    onAssignFolderToGroup, onReorderFolders, onUpdateGroupOrder, onCreateGroup, onUpdateGroup, onDeleteGroup,
    activeView = 'files', onViewChange,
}: SidebarProps) {
    const [showNewFolderInput, setShowNewFolderInput] = useState(false);
    const [newFolderName, setNewFolderName] = useState("");
    const { t } = useTranslation();
    const { settings, updateSetting } = useSettings();

    // Grouping States
    const [activeGroupId, setActiveGroupId] = useState<number | null | 'all'>('all');
    const [showGroupEditor, setShowGroupEditor] = useState(false);
    const [editingGroup, setEditingGroup] = useState<FolderGroup | null>(null); // null means creating
    const [groupName, setGroupName] = useState("");
    const [groupColor, setGroupColor] = useState("#3B82F6");

    // DND Kit Sensors
    const sensors = useSensors(
        useSensor(PointerSensor, {
            activationConstraint: {
                distance: 8,
            },
        }),
        useSensor(KeyboardSensor, {
            coordinateGetter: sortableKeyboardCoordinates,
        })
    );

    const handleDragEnd = (event: DragEndEvent) => {
        const { active, over } = event;
        if (!over) return;

        const activeId = active.id.toString();
        const overId = over.id.toString();

        if (activeId.startsWith('folder-')) {
            const activeFolderId = parseInt(activeId.replace('folder-', ''), 10);

            if (overId.startsWith('group-tab-')) {
                const overGroupIdStr = overId.replace('group-tab-', '');
                const overGroupId = overGroupIdStr === 'all'
                    ? null
                    : overGroupIdStr === 'unassigned'
                        ? null
                        : parseInt(overGroupIdStr, 10);
                onAssignFolderToGroup(activeFolderId, overGroupId);
            } else if (overId.startsWith('folder-')) {
                const overFolderId = parseInt(overId.replace('folder-', ''), 10);
                if (activeFolderId !== overFolderId) {
                    const oldIndex = folders.findIndex(f => f.id === activeFolderId);
                    const newIndex = folders.findIndex(f => f.id === overFolderId);
                    if (oldIndex !== -1 && newIndex !== -1) {
                        const reordered = arrayMove(folders, oldIndex, newIndex);
                        onReorderFolders(reordered);
                    }
                }
            }
        } else if (activeId.startsWith('group-tab-')) {
            const activeGroupId = parseInt(activeId.replace('group-tab-', ''), 10);

            if (overId.startsWith('group-tab-')) {
                const overGroupIdStr = overId.replace('group-tab-', '');
                if (overGroupIdStr !== 'all' && overGroupIdStr !== 'unassigned') {
                    const overGroupId = parseInt(overGroupIdStr, 10);
                    if (activeGroupId !== overGroupId) {
                        const oldIndex = groups.findIndex(g => g.id === activeGroupId);
                        const newIndex = groups.findIndex(g => g.id === overGroupId);
                        if (oldIndex !== -1 && newIndex !== -1) {
                            const reordered = arrayMove(groups, oldIndex, newIndex);
                            onUpdateGroupOrder(reordered);
                        }
                    }
                }
            }
        }
    };

    const submitCreate = async () => {
        if (!newFolderName.trim()) return;
        try {
            await onCreate(newFolderName);
            setNewFolderName("");
            setShowNewFolderInput(false);
        } catch {
            // handled by parent
        }
    };

    const handleSaveGroup = async () => {
        if (!groupName.trim()) return;
        if (editingGroup) {
            await onUpdateGroup(editingGroup.id, groupName, groupColor);
        } else {
            await onCreateGroup(groupName, groupColor);
        }
        setShowGroupEditor(false);
        setEditingGroup(null);
        setGroupName("");
        setGroupColor("#3B82F6");
    };

    const handleDeleteGroupClick = async (groupId: number) => {
        await onDeleteGroup(groupId);
        if (activeGroupId === groupId) {
            setActiveGroupId('all');
        }
        setShowGroupEditor(false);
        setEditingGroup(null);
        setGroupName("");
        setGroupColor("#3B82F6");
    };

    const filteredFolders = folders.filter(folder => {
        if (settings.hideGroups || activeGroupId === 'all') return true;
        if (activeGroupId === null) return folder.group_id === null || folder.group_id === undefined;
        return folder.group_id === activeGroupId;
    });

    return (
        <aside 
            className={`transition-all duration-300 ${settings.sidebarCollapsed ? 'w-14' : 'w-64'} bg-telegram-surface border-r border-telegram-border flex flex-col`} 
            onClick={e => e.stopPropagation()}
        >
            <div className={`p-4 flex ${settings.sidebarCollapsed ? 'flex-col items-center gap-2' : 'items-center justify-between'} min-h-[64px]`}>
                <div className="flex items-center gap-2">
                    <img src="/logo.svg" className="w-8 h-8 drop-shadow-lg" alt="Logo" />
                    {!settings.sidebarCollapsed && (
                        <span className="font-bold text-lg text-telegram-text tracking-tight">{t('common.app_title')}</span>
                    )}
                </div>
                <button
                    onClick={() => updateSetting('sidebarCollapsed', !settings.sidebarCollapsed)}
                    className="p-1 rounded-md hover:bg-telegram-hover text-telegram-subtext hover:text-telegram-text transition-colors"
                    title={settings.sidebarCollapsed ? t('common.expand_sidebar') || "Expand Sidebar" : t('common.collapse_sidebar') || "Collapse Sidebar"}
                >
                    {settings.sidebarCollapsed ? <ChevronRight className="w-4.5 h-4.5" /> : <ChevronLeft className="w-4.5 h-4.5" />}
                </button>
            </div>

            <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={handleDragEnd}
            >
                {!settings.sidebarCollapsed && (
                    <div className="px-4 py-2 border-b border-telegram-border flex flex-col gap-2">
                        <div className="flex items-center justify-between">
                            <span className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-1.5">
                                {t('common.groups') || "Groups"}
                            </span>
                            <div className="flex items-center gap-1">
                                <button
                                    onClick={() => updateSetting('hideGroups', !settings.hideGroups)}
                                    className="p-1 rounded-md hover:bg-telegram-hover text-telegram-subtext hover:text-telegram-text transition-all"
                                    title={settings.hideGroups ? t('common.show_groups') || "Show Groups" : t('common.hide_groups') || "Hide Groups"}
                                >
                                    {settings.hideGroups ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
                                </button>
                                {!settings.hideGroups && (
                                    <button
                                        onClick={() => {
                                            setEditingGroup(null);
                                            setGroupName("");
                                            setGroupColor("#3B82F6");
                                            setShowGroupEditor(true);
                                        }}
                                        className="p-1 rounded-md hover:bg-telegram-hover text-telegram-subtext hover:text-telegram-text transition-all"
                                        title={t('common.create_group') || "Create Group"}
                                    >
                                        <Plus className="w-3.5 h-3.5" />
                                    </button>
                                )}
                            </div>
                        </div>

                        {!settings.hideGroups && showGroupEditor && (
                            <div className="p-3 bg-telegram-hover/50 rounded-lg border border-telegram-border flex flex-col gap-3 animate-in fade-in slide-in-from-top-1 duration-150">
                                <div>
                                    <label className="text-[10px] font-semibold text-telegram-subtext uppercase tracking-wider block mb-1">
                                        {editingGroup ? t('common.edit_group_name') : t('common.new_group_name')}
                                    </label>
                                    <input
                                        autoFocus
                                        type="text"
                                        className="w-full bg-white/10 rounded px-2 py-1 text-xs text-telegram-text focus:outline-none focus:ring-1 focus:ring-telegram-primary"
                                        placeholder={t('common.enter_group_name')}
                                        value={groupName}
                                        onChange={e => setGroupName(e.target.value)}
                                    />
                                </div>

                                <div>
                                    <label className="text-[10px] font-semibold text-telegram-subtext uppercase tracking-wider block mb-1">
                                        {t('common.theme_color')}
                                    </label>
                                    <div className="flex flex-wrap gap-1.5">
                                        {PRESET_COLORS.map(color => (
                                            <button
                                                key={color}
                                                onClick={() => setGroupColor(color)}
                                                className={`w-5 h-5 rounded-full border transition-all ${
                                                    groupColor === color
                                                        ? 'border-white scale-110 shadow-md ring-1 ring-telegram-primary'
                                                        : 'border-transparent hover:scale-105'
                                                }`}
                                                style={{ backgroundColor: color }}
                                            />
                                        ))}
                                    </div>
                                </div>

                                <div className="flex gap-2 justify-end mt-1">
                                    {editingGroup && (
                                        <button
                                            onClick={() => handleDeleteGroupClick(editingGroup.id)}
                                            className="mr-auto p-1.5 text-red-500 hover:bg-red-500/10 rounded transition-colors"
                                            title={t('common.delete_group')}
                                        >
                                            <Trash2 className="w-3.5 h-3.5" />
                                        </button>
                                    )}
                                    <button
                                        onClick={() => {
                                            setShowGroupEditor(false);
                                            setEditingGroup(null);
                                        }}
                                        className="px-2 py-1 text-[11px] font-semibold text-telegram-subtext hover:bg-telegram-hover rounded transition-colors flex items-center gap-1"
                                    >
                                        <X className="w-3 h-3" />
                                        {t('common.cancel') || "Cancel"}
                                    </button>
                                    <button
                                        onClick={handleSaveGroup}
                                        disabled={!groupName.trim()}
                                        className="px-2.5 py-1 text-[11px] font-semibold bg-telegram-primary text-white hover:bg-telegram-primary/80 rounded transition-colors flex items-center gap-1 disabled:opacity-50"
                                    >
                                        <Check className="w-3 h-3" />
                                        {t('common.save') || "Save"}
                                    </button>
                                </div>
                            </div>
                        )}

                        {!settings.hideGroups && (
                            <div 
                                className="group-tabs-scroll flex items-center gap-2 overflow-x-auto py-1"
                                style={{ scrollbarWidth: 'none', msOverflowStyle: 'none' }}
                            >
                                <style>{`
                                    .group-tabs-scroll::-webkit-scrollbar {
                                        display: none;
                                    }
                                `}</style>
                                <GroupTab
                                    id="group-tab-all"
                                    groupId="all"
                                    label={t('common.all') || "All"}
                                    active={activeGroupId === 'all'}
                                    onClick={() => setActiveGroupId('all')}
                                    isSortable={false}
                                />
                                <GroupTab
                                    id="group-tab-unassigned"
                                    groupId={null}
                                    label={t('common.unassigned') || "Unassigned"}
                                    active={activeGroupId === null}
                                    onClick={() => setActiveGroupId(null)}
                                    isSortable={false}
                                />
                                <SortableContext
                                    items={groups.map(g => `group-tab-${g.id}`)}
                                    strategy={horizontalListSortingStrategy}
                                >
                                    {groups.map(group => (
                                        <GroupTab
                                            key={group.id}
                                            id={`group-tab-${group.id}`}
                                            groupId={group.id}
                                            label={group.name}
                                            colorHex={group.color_hex}
                                            active={activeGroupId === group.id}
                                            onClick={() => setActiveGroupId(group.id)}
                                            onEdit={() => {
                                                setEditingGroup(group);
                                                setGroupName(group.name);
                                                setGroupColor(group.color_hex || "#3B82F6");
                                                setShowGroupEditor(true);
                                            }}
                                        />
                                    ))}
                                </SortableContext>
                            </div>
                        )}
                    </div>
                )}

                {/* Scrollable folder list */}
                <nav className="flex-1 px-2 py-4 space-y-1 overflow-y-auto min-h-0">
                    <SidebarItem
                        icon={HardDrive}
                        label={t('common.saved_messages')}
                        active={activeView === 'files' && activeFolderId === null}
                        onClick={() => {
                            onViewChange?.('files');
                            setActiveFolderId(null);
                        }}
                        onDrop={(e: React.DragEvent) => onDrop(e, null)}
                        folderId={null}
                        collapsed={settings.sidebarCollapsed}
                    />
                    <SidebarItem
                        icon={Cloud}
                        label={t('migration.page_title', 'OneDrive Migration')}
                        active={activeView === 'onedrive'}
                        onClick={() => onViewChange?.('onedrive')}
                        folderId={null}
                        collapsed={settings.sidebarCollapsed}
                    />
                    <SortableContext
                        items={filteredFolders.map(folder => `folder-${folder.id}`)}
                        strategy={verticalListSortingStrategy}
                    >
                        {filteredFolders.map(folder => (
                            <SidebarItem
                                key={folder.id}
                                icon={Folder}
                                label={folder.name}
                                active={activeView === 'files' && activeFolderId === folder.id}
                                onClick={() => {
                                    onViewChange?.('files');
                                    setActiveFolderId(folder.id);
                                }}
                                onDrop={(e: React.DragEvent) => onDrop(e, folder.id)}
                                onDelete={() => onDelete(folder.id, folder.name)}
                                onRename={() => onRename(folder.id, folder.name)}
                                onToggleVisibility={() => onToggleVisibility(folder.id, folder.name, !!(folder.is_public || folder.username))}
                                onExportInvite={() => onExportInvite(folder.id, folder.name)}
                                folderId={folder.id}
                                isPublic={!!(folder.is_public || folder.username)}
                                collapsed={settings.sidebarCollapsed}
                                groups={groups}
                                onAssignFolderToGroup={onAssignFolderToGroup}
                            />
                        ))}
                    </SortableContext>
                </nav>
            </DndContext>

            {/* Sticky Create Folder section — always visible above the footer */}
            {!settings.sidebarCollapsed && (
                <div className="px-2 pb-2 border-b border-telegram-border">
                    {showNewFolderInput ? (
                        <div className="px-3 py-2">
                            <input
                                autoFocus
                                type="text"
                                className="w-full bg-white/10 rounded px-2 py-1 text-sm text-telegram-text focus:outline-none focus:ring-1 focus:ring-telegram-primary"
                                placeholder={t('common.folder_name_placeholder')}
                                value={newFolderName}
                                onChange={e => setNewFolderName(e.target.value)}
                                onKeyDown={e => e.key === 'Enter' && submitCreate()}
                                onBlur={() => !newFolderName && setShowNewFolderInput(false)}
                            />
                        </div>
                    ) : (
                        <button
                            onClick={() => setShowNewFolderInput(true)}
                            className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text transition-colors border border-dashed border-telegram-border"
                        >
                            <Plus className="w-4 h-4" />
                            {t('common.create_folder')}
                        </button>
                    )}
                </div>
            )}

            <div className={`p-4 border-t border-telegram-border flex flex-col ${settings.sidebarCollapsed ? 'items-center gap-4' : 'gap-4'}`}>
                {settings.sidebarCollapsed ? (
                    <>
                        <div
                            className={`w-2.5 h-2.5 rounded-full flex-shrink-0 ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`}
                            title={isConnected ? t('common.connected_telegram') : t('common.disconnected_telegram')}
                        />
                        <button
                            onClick={onSync}
                            disabled={isSyncing}
                            className={`p-2 text-blue-500 hover:text-blue-600 bg-blue-500/10 hover:bg-blue-500/20 rounded-lg transition-colors ${isSyncing ? 'opacity-50 cursor-not-allowed' : ''}`}
                            title={isSyncing ? t('common.syncing') : t('common.sync')}
                        >
                            <RefreshCw className={`w-4 h-4 ${isSyncing ? 'animate-spin' : ''}`} />
                        </button>
                        <button
                            onClick={onLogout}
                            className="p-2 text-red-500 hover:text-red-600 bg-red-500/10 hover:bg-red-500/20 rounded-lg transition-colors"
                            title={t('common.logout')}
                        >
                            <LogOut className="w-4 h-4" />
                        </button>
                    </>
                ) : (
                    <>
                        <div className="flex items-center gap-2 text-telegram-subtext text-xs">
                            <div className={`w-2 h-2 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`}></div>
                            <span>{isConnected ? t('common.connected_telegram') : t('common.disconnected_telegram')}</span>
                        </div>

                        <div className="flex gap-2">
                            <button
                                onClick={onSync}
                                disabled={isSyncing}
                                className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-blue-500 hover:text-blue-600 bg-blue-500/10 hover:bg-blue-500/20 rounded-lg transition-colors ${isSyncing ? 'opacity-50 cursor-not-allowed' : ''}`}
                                title="Scan for existing folders"
                            >
                                <RefreshCw className={`w-3 h-3 ${isSyncing ? 'animate-spin' : ''}`} />
                                {isSyncing ? t('common.syncing') : t('common.sync')}
                            </button>
                            <button
                                onClick={onLogout}
                                className="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-red-500 hover:text-red-600 bg-red-500/10 hover:bg-red-500/20 rounded-lg transition-colors"
                                title="Sign Out"
                            >
                                <LogOut className="w-3 h-3" />
                                {t('common.logout')}
                            </button>
                        </div>

                        {bandwidth && <BandwidthWidget bandwidth={bandwidth} />}
                    </>
                )}
            </div>
        </aside>
    );
}
