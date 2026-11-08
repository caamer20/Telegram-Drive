import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    Download,
    FolderInput,
    FolderPlus,
    Filter,
    Globe,
    HardDrive,
    HelpCircle,
    LayoutGrid,
    List,
    Moon,
    MoreHorizontal,
    Settings,
    Share2,
    Keyboard,
    Sun,
    Trash2,
    UploadCloud,
    X,
    ZoomIn,
    ZoomOut,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useTheme } from '../../../context/ThemeContext';
import { useSettings } from '../../../context/SettingsContext';
import { Button, IconButton, MenuItem, MenuPanel, SearchField, Select } from '../../ui';
import type { SortDirection, SortField } from './FileExplorer';
import { DEFAULT_SEARCH_FILTERS, type FileSearchFilters } from '../../../services/fileSearch';
import { stopControlShortcuts, useTopBarController } from './useTopBarController';

const toolbarIcon = 'h-3.5 w-3.5';
const menuIcon = `${toolbarIcon} text-app-text-secondary`;
const filterField = 'search-filter-field text-xs font-medium text-app-text-secondary';
const filterPill = (active: boolean) => `quiet-control min-h-8 border px-3 text-badge font-medium ${active ? 'border-app-accent/25 bg-app-selected text-app-accent' : 'border-app-border text-app-text-secondary hover:text-app-text'}`;

interface TopBarProps {
    currentFolderName: string;
    fileCount?: number;
    selectedIds: number[];
    onShowMoveModal: () => void;
    onBulkDownload: () => void;
    onBulkDelete: () => void;
    onBulkShare: () => void;
    onDownloadFolder: () => void;
    onClearSelection: () => void;
    onUploadClick: () => void;
    viewMode: 'grid' | 'list';
    setViewMode: (mode: 'grid' | 'list') => void;
    cardScale: number;
    onCardScaleChange: (scale: number) => void;
    sortField: SortField;
    sortDirection: SortDirection;
    onSortChange: (field: SortField) => void;
    searchTerm: string;
    onSearchChange: (term: string) => void;
    onSettingsClick: () => void;
    onRemoteUploadClick: () => void;
    onNewFolderClick: () => void;
    onShowShortcuts: () => void;
    onShowHelp: () => void;
    searchFilters: FileSearchFilters;
    onSearchFiltersChange: (filters: FileSearchFilters) => void;
}

export function TopBar({
    currentFolderName,
    fileCount,
    selectedIds,
    onShowMoveModal,
    onBulkDownload,
    onBulkDelete,
    onBulkShare,
    onDownloadFolder,
    onClearSelection,
    onUploadClick,
    viewMode,
    setViewMode,
    cardScale,
    onCardScaleChange,
    sortField,
    sortDirection,
    onSortChange,
    searchTerm,
    onSearchChange,
    onSettingsClick,
    onRemoteUploadClick,
    onNewFolderClick,
    onShowShortcuts,
    onShowHelp,
    searchFilters,
    onSearchFiltersChange,
}: TopBarProps) {
    const { theme, toggleTheme } = useTheme();
    const { t } = useTranslation();
    const { settings } = useSettings();
    const {
        proxyStatus,
        showMore,
        showViewOptions,
        showSearchFilters,
        moreRef,
        viewRef,
        filterRef,
        filterButtonRef,
        closeSearchFilters,
        onFilterKeyDown,
        toggleSearchFilters,
        toggleViewOptions,
        toggleMore,
        runMoreAction,
    } = useTopBarController(settings.proxyEnabled, settings.proxyLiveStateEnabled);
    const hasSearchFilters = Object.entries(searchFilters).some(([key, value]) => value !== DEFAULT_SEARCH_FILTERS[key as keyof FileSearchFilters]);
    const hasSelection = selectedIds.length > 0;

    const sortLabel = t(`common.${sortField}`);
    const searchScopeLabel = searchFilters.scope === 'all' ? t('workspace.all_folders') : currentFolderName;
    const searchLabel = `${t('common.search_placeholder')} · ${searchScopeLabel}`;

    return (
        <>
            <header
                className="desktop-chrome-row quiet-toolbar sticky top-0 z-20 gap-2.5 px-3"
                onClick={(event) => event.stopPropagation()}
                onKeyDown={stopControlShortcuts}
            >
                <div className="flex min-w-0 flex-[0_1_12rem] items-center gap-1.5 text-ui">
                    <span className="shrink-0 text-app-text-tertiary">{t('common.files')}</span>
                    <span className="shrink-0 text-app-text-tertiary" aria-hidden="true">/</span>
                    <span className="truncate font-medium text-app-text" title={currentFolderName}>{currentFolderName}</span>
                </div>
                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                    <SearchField
                        data-file-search
                        aria-label={searchLabel}
                        containerClassName="min-w-0 flex-1"
                        placeholder={searchLabel}
                        value={searchTerm}
                        onChange={(event) => onSearchChange(event.target.value)}
                    />
                    <div className="relative shrink-0" onKeyDown={onFilterKeyDown}>
                        <IconButton
                            ref={filterButtonRef}
                            data-file-filter-trigger
                            label={t('common.search_filters')}
                            size="lg"
                            aria-haspopup="dialog"
                            aria-controls={showSearchFilters ? 'search-filter-panel' : undefined}
                            data-active={hasSearchFilters}
                            onClick={toggleSearchFilters}
                            aria-expanded={showSearchFilters}
                            className="search-filter-trigger"
                        >
                            <Filter className={toolbarIcon} />
                        </IconButton>

                    </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                    {settings.proxyEnabled && settings.proxyLiveStateEnabled && (
                        <div className="quiet-control hidden h-7 items-center gap-1.5 px-2 text-badge text-app-text-secondary xl:flex" title={!proxyStatus ? 'Proxy status: checking…' : proxyStatus.reachable ? `Proxy active: ${proxyStatus.latency_ms}ms latency` : 'Proxy status: unreachable'}>
                            <span className={`h-1.5 w-1.5 rounded-full ${!proxyStatus ? 'bg-app-warning animate-pulse' : proxyStatus.reachable ? 'bg-app-success' : 'bg-app-danger'}`} />
                            <span className="font-mono">{!proxyStatus ? '…' : proxyStatus.reachable ? `${proxyStatus.latency_ms}ms` : '—'}</span>
                        </div>
                    )}
                    <Button variant="primary" onClick={onUploadClick} leadingIcon={<UploadCloud className={toolbarIcon} />} className="toolbar-upload-action">{t('common.upload')}</Button>
                    <div className="relative" ref={moreRef}>
                        <IconButton label={t('common.preferences')} onClick={toggleMore} aria-expanded={showMore}>
                            <MoreHorizontal className={toolbarIcon} />
                        </IconButton>
                        {showMore && (
                            <MenuPanel className="absolute end-0 top-9 z-50 w-56">
                                <MenuItem onClick={() => runMoreAction(onNewFolderClick)}><FolderPlus className={menuIcon} />{t('common.create_folder')}</MenuItem>
                                <MenuItem onClick={() => runMoreAction(onDownloadFolder)}><HardDrive className={menuIcon} />{t('files.download_folder')}</MenuItem>
                                <MenuItem onClick={() => runMoreAction(onRemoteUploadClick)}><Globe className={menuIcon} />{t('files.remote_upload')}</MenuItem>
                                <MenuItem onClick={() => runMoreAction(toggleTheme)}>{theme === 'dark' ? <Sun className={menuIcon} /> : <Moon className={menuIcon} />}{theme === 'dark' ? t('common.light_mode') : t('common.dark_mode')}</MenuItem>
                                <MenuItem onClick={() => runMoreAction(onShowShortcuts)}><Keyboard className={menuIcon} />{t('ui_copy.keyboard_shortcuts')}</MenuItem>
                                <MenuItem onClick={() => runMoreAction(onShowHelp)}><HelpCircle className={menuIcon} />{t('ui_copy.help_faq')}</MenuItem>
                                <div className="my-1 h-px bg-app-border-subtle" />
                                <MenuItem onClick={() => runMoreAction(onSettingsClick)}><Settings className={menuIcon} />{t('common.preferences')}</MenuItem>
                            </MenuPanel>
                        )}
                    </div>
                </div>
            </header>

            <section className="space-y-3 border-b border-app-border-subtle px-4 pb-3 pt-4 md:px-6" aria-label={t('common.files')} onKeyDown={stopControlShortcuts}>
                <div className="flex flex-wrap items-end justify-between gap-3">
                    <div className="min-w-0">
                        <h1 className="truncate text-xl font-semibold tracking-[-0.01em] text-app-text" title={currentFolderName}>{currentFolderName}</h1>
                        <p className="mt-1 text-metadata text-app-text-secondary">
                            {fileCount === undefined ? t(`common.${sortField}`) : `${t('workspace.result_count', { count: fileCount })} · ${sortLabel}`}
                            <span className="ms-1" aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>
                        </p>
                    </div>
                    <div className="relative flex flex-wrap items-center gap-1.5" onKeyDown={onFilterKeyDown}>
                        <button type="button" onClick={() => onSearchFiltersChange(DEFAULT_SEARCH_FILTERS)} className={filterPill(!hasSearchFilters)} aria-pressed={!hasSearchFilters}>
                            {t('common.all')} {t('common.files')}
                        </button>
                        {([
                            ['type', searchFilters.type !== 'all', searchFilters.type === 'all' ? t('workspace.file_type') : t(`workspace.types.${searchFilters.type}`)],
                            ['date', searchFilters.date !== 'any', searchFilters.date === 'any' ? t('common.date') : t(`workspace.dates.${searchFilters.date}`)],
                            ['size', true, searchFilters.size !== 'any' && t(`workspace.sizes.${searchFilters.size}`)],
                            ['scope', true, searchFilters.scope === 'all' && t('workspace.all_folders')],
                        ] as const).map(([key, active, label]) => label && <button key={key} type="button" data-file-filter-trigger onClick={toggleSearchFilters} className={filterPill(active)} aria-expanded={showSearchFilters}>{label}</button>)}
                        <div className="ms-1 flex items-center rounded-control border border-app-border bg-app-surface-sunken/45 p-0.5" role="group" aria-label={`${t('files.switch_list')} / ${t('files.switch_grid')}`}>
                            <IconButton label={t('files.switch_list')} aria-pressed={viewMode === 'list'} className={viewMode === 'list' ? 'bg-app-surface-raised text-app-text shadow-sm' : ''} onClick={() => setViewMode('list')}><List className={toolbarIcon} /></IconButton>
                            <IconButton label={t('files.switch_grid')} aria-pressed={viewMode === 'grid'} className={viewMode === 'grid' ? 'bg-app-surface-raised text-app-text shadow-sm' : ''} onClick={() => setViewMode('grid')}><LayoutGrid className={toolbarIcon} /></IconButton>
                        </div>
                        <div className="relative" ref={viewRef}>
                            <IconButton label={t('ui_copy.sort_files')} onClick={toggleViewOptions} aria-expanded={showViewOptions} className={showViewOptions ? 'bg-app-selected text-app-accent' : ''}><ArrowUpDown className={toolbarIcon} /></IconButton>
                            {showViewOptions && (
                                <MenuPanel className="absolute end-0 top-9 z-50 w-64">
                                    <div className="px-2 pb-2 pt-1 text-badge font-medium text-app-text-tertiary">{t('ui_copy.sort_files')}</div>
                                    <div className="grid grid-cols-3 gap-1" role="group" aria-label={t('ui_copy.sort_files')}>
                                        {(['name', 'size', 'date'] as const).map(field => (
                                            <button key={field} type="button" aria-pressed={sortField === field} onClick={() => onSortChange(field)} className={`quiet-control flex h-8 min-w-0 items-center justify-center gap-1 px-1.5 text-metadata font-medium ${sortField === field ? 'bg-app-selected text-app-accent' : 'text-app-text-secondary hover:text-app-text'}`}>
                                                <span className="truncate">{t(`common.${field}`)}</span>
                                                {sortField === field && (sortDirection === 'asc' ? <ArrowUp className="h-3 w-3 shrink-0" /> : <ArrowDown className="h-3 w-3 shrink-0" />)}
                                            </button>
                                        ))}
                                    </div>
                                    {viewMode === 'grid' && <>
                                        <div className="my-1 h-px bg-app-border-subtle" />
                                        <div className="flex h-8 items-center gap-1 px-1">
                                            <IconButton size="xs" label="Smaller thumbnails" onClick={() => onCardScaleChange(Math.max(0.5, cardScale - 0.25))} disabled={cardScale <= 0.5}><ZoomOut className={toolbarIcon} /></IconButton>
                                            <input type="range" min="0.5" max="2" step="0.25" value={cardScale} onChange={event => onCardScaleChange(parseFloat(event.target.value))} className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-app-border [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-app-accent" aria-label={t('ui_copy.thumbnail_size')} />
                                            <IconButton size="xs" label="Larger thumbnails" onClick={() => onCardScaleChange(Math.min(2, cardScale + 0.25))} disabled={cardScale >= 2}><ZoomIn className={toolbarIcon} /></IconButton>
                                            <span className="w-9 text-end text-badge tabular-nums text-app-text-tertiary">{Math.round(cardScale * 100)}%</span>
                                        </div>
                                    </>}
                                </MenuPanel>
                            )}
                        </div>
                        {showSearchFilters && (
                            <div ref={filterRef} id="search-filter-panel" role="dialog" aria-label={t('common.search_filters')} className="quiet-menu search-filter-panel absolute end-0 top-full z-50 mt-2">
                                <div className="flex items-center justify-between border-b border-app-border-subtle px-4 py-2">
                                    <span className="text-ui font-semibold text-app-text">{t('common.search_filters')}</span>
                                    <IconButton size="xs" label={t('common.close')} onClick={closeSearchFilters}><X className={toolbarIcon} /></IconButton>
                                </div>
                                <div className="grid gap-3 p-4">
                                    <label className={filterField}>{t('ui_copy.search_scope')}
                                        <Select value={searchFilters.scope} onChange={(event) => onSearchFiltersChange({ ...searchFilters, scope: event.target.value as FileSearchFilters['scope'] })} className="search-filter-select">
                                            <option value="folder">{t('ui_copy.current_scope')}</option>
                                            <option value="all">{t('ui_copy.global_scope')}</option>
                                        </Select>
                                    </label>
                                    <label className={filterField}>{t('workspace.file_type')}
                                        <Select value={searchFilters.type} onChange={(event) => onSearchFiltersChange({ ...searchFilters, type: event.target.value as FileSearchFilters['type'] })} className="search-filter-select">
                                            <option value="all">{t('workspace.types.all')}</option><option value="image">{t('workspace.types.image')}</option><option value="video">{t('workspace.types.video')}</option><option value="audio">{t('workspace.types.audio')}</option><option value="document">{t('workspace.types.document')}</option><option value="archive">{t('workspace.types.archive')}</option><option value="other">{t('workspace.types.other')}</option>
                                        </Select>
                                    </label>
                                    <label className={filterField}>{t('common.size')}
                                        <Select value={searchFilters.size} onChange={(event) => onSearchFiltersChange({ ...searchFilters, size: event.target.value as FileSearchFilters['size'] })} className="search-filter-select">
                                            <option value="any">{t('workspace.sizes.any')}</option><option value="small">{t('workspace.sizes.small')}</option><option value="medium">{t('workspace.sizes.medium')}</option><option value="large">{t('workspace.sizes.large')}</option>
                                        </Select>
                                    </label>
                                    <label className={filterField}>{t('common.date')}
                                        <Select value={searchFilters.date} onChange={(event) => onSearchFiltersChange({ ...searchFilters, date: event.target.value as FileSearchFilters['date'] })} className="search-filter-select">
                                            <option value="any">{t('workspace.sizes.any')}</option><option value="7d">{t('workspace.dates.7d')}</option><option value="30d">{t('workspace.dates.30d')}</option><option value="1y">{t('workspace.dates.1y')}</option>
                                        </Select>
                                    </label>
                                        <label className="search-filter-field text-xs font-medium text-app-text-secondary">{t('workspace.protection')}
                                            <Select aria-label={t('workspace.protection')} value={searchFilters.protection ?? 'any'} onChange={event => onSearchFiltersChange({ ...searchFilters, protection: event.target.value as FileSearchFilters['protection'] })} className="search-filter-select">
                                                <option value="any">{t('common.all')}</option><option value="plain">{t('workspace.protection_plain')}</option><option value="protected">{t('settings.protected')}</option><option value="locked">{t('workspace.protection_locked')}</option><option value="unlocked">{t('workspace.protection_unlocked')}</option>
                                            </Select>
                                        </label>
                                </div>
                                <div className="border-t border-app-border-subtle p-3">
                                    <Button className="w-full" onClick={() => onSearchFiltersChange(DEFAULT_SEARCH_FILTERS)}>{t('ui_copy.reset_filters')}</Button>
                                </div>
                            </div>
                        )}
                    </div>
                </div>
                {hasSelection && <div className="flex flex-wrap items-center gap-2 rounded-control border border-app-accent/20 bg-app-selected/60 px-3 py-2" aria-label={t('workspace.selected_count', { count: selectedIds.length })}>
                        <span className="me-auto min-w-0 truncate text-ui font-semibold text-app-accent">{t('workspace.selected_count', { count: selectedIds.length })}</span>
                        <Button size="sm" onClick={onBulkDownload} leadingIcon={<Download className={toolbarIcon} />}>{t('files.download')}</Button>
                        <Button size="sm" onClick={onShowMoveModal} leadingIcon={<FolderInput className={toolbarIcon} />}>{t('files.move_to')}</Button>
                        <Button size="sm" onClick={onBulkShare} leadingIcon={<Share2 className={toolbarIcon} />}>{t('files.share')}</Button>
                        <Button size="sm" variant="ghost" onClick={onClearSelection}>{t('files.clear_selection')}</Button>
                        <Button size="sm" variant="danger" onClick={onBulkDelete} leadingIcon={<Trash2 className={toolbarIcon} />}>{t('files.delete')}</Button>
                </div>}
            </section>
        </>
    );
}
