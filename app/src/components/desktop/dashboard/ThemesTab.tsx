import { useState } from 'react';
import { Check, Monitor, Moon, Palette, Plus, RotateCcw, Sparkles, Sun, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useConfirm } from '../../../context/ConfirmContext';
import { useTheme } from '../../../context/ThemeContext';
import { CustomTheme, ThemeColorPalette, generateThemeId } from '../../../theme/themeEngine';
import { getDefaultPalette } from '../../../theme/presets';

const builtinThemeLabelKeys: Record<string, string> = {
    'default-dark': 'theme_names.default_dark',
    'charcoal': 'theme_names.charcoal',
    'cyber-teal': 'theme_names.cyber_teal',
    'default-light': 'theme_names.default_light',
    'ocean': 'theme_names.ocean',
    'forest': 'theme_names.forest',
    'plum': 'theme_names.plum',
    'paper': 'theme_names.paper',
    'rose': 'theme_names.rose',
    'mint': 'theme_names.mint',
    'sky': 'theme_names.sky',
};

const PALETTE_KEYS: { key: keyof ThemeColorPalette; labelKey: string }[] = [
    { key: 'bg', labelKey: 'settings.color_bg' },
    { key: 'surface', labelKey: 'settings.color_surface' },
    { key: 'primary', labelKey: 'settings.color_primary' },
    { key: 'secondary', labelKey: 'settings.color_secondary' },
    { key: 'text', labelKey: 'settings.color_text' },
    { key: 'subtext', labelKey: 'settings.color_subtext' },
    { key: 'border', labelKey: 'settings.color_border' },
    { key: 'hover', labelKey: 'settings.color_hover' },
];

export function ThemesTab() {
    const { t } = useTranslation();
    const themeName = (theme: CustomTheme) => theme.isBuiltin && builtinThemeLabelKeys[theme.id] ? t(builtinThemeLabelKeys[theme.id]) : theme.name;
    const {
        customThemes,
        themePreference,
        setThemePreference,
        activeCustomThemeId,
        setActiveCustomTheme,
        addCustomTheme,
        deleteCustomTheme,
        updateCustomTheme,
    } = useTheme();
    const { confirm } = useConfirm();
    const [editingId, setEditingId] = useState<string | null>(() =>
        customThemes.some(theme => theme.id === activeCustomThemeId && !theme.isBuiltin) ? activeCustomThemeId : null
    );

    const builtinThemes = customThemes.filter(theme => theme.isBuiltin);
    const userThemes = customThemes.filter(theme => !theme.isBuiltin);
    const editingTheme = editingId ? customThemes.find(theme => theme.id === editingId) : null;

    const handleCreateTheme = () => {
        const id = generateThemeId();
        const newTheme: CustomTheme = {
            id,
            name: 'My Theme',
            isDark: true,
            palette: getDefaultPalette(true),
        };
        addCustomTheme(newTheme);
        setEditingId(id);
        setActiveCustomTheme(id);
    };

    const handleSelectTheme = (theme: CustomTheme) => {
        if (activeCustomThemeId === theme.id) {
            setActiveCustomTheme(null);
            setEditingId(null);
        } else {
            setActiveCustomTheme(theme.id);
            if (!theme.isBuiltin) {
                setEditingId(theme.id);
            } else {
                setEditingId(null);
            }
        }
    };

    const handleDeleteTheme = async (id: string) => {
        const ok = await confirm({
            title: t('settings.delete_theme'),
            message: t('settings.delete_theme_confirm'),
            confirmText: t('common.delete'),
            variant: 'danger',
        });
        if (!ok) return;
        deleteCustomTheme(id);
        if (editingId === id) setEditingId(null);
    };

    const handlePaletteChange = (key: keyof ThemeColorPalette, value: string) => {
        if (!editingTheme || editingTheme.isBuiltin) return;
        const newPalette = { ...editingTheme.palette, [key]: value };
        updateCustomTheme(editingTheme.id, { palette: newPalette });
    };

    const handleBaseToggle = (isDark: boolean) => {
        if (!editingTheme || editingTheme.isBuiltin) return;
        updateCustomTheme(editingTheme.id, { isDark });
    };

    const handleNameChange = (name: string) => {
        if (!editingTheme || editingTheme.isBuiltin) return;
        updateCustomTheme(editingTheme.id, { name });
    };

    return (
        <section
            className="w-full space-y-6"
        >
            <div className="space-y-2">
                <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-app-text-secondary">
                    <Monitor className="h-3.5 w-3.5" />
                    {t('common.theme')}
                </h3>
                <div role="group" aria-label={t('common.theme')} className="grid grid-cols-2 gap-2 sm:grid-cols-4 rounded-container border border-app-border-subtle bg-app-surface-sunken/30 p-2">
                    {([
                        ['default', Sparkles, t('common.default', { defaultValue: 'Default' })],
                        ['system', Monitor, t('common.system')],
                        ['light', Sun, t('common.light_mode')],
                        ['dark', Moon, t('common.dark_mode')],
                    ] as const).map(([preference, Icon, label]) => (
                        <button
                            key={preference}
                            type="button"
                            aria-pressed={!activeCustomThemeId && themePreference === preference}
                            onClick={() => setThemePreference(preference)}
                            className={`quiet-control flex items-center justify-center gap-2 border px-3 py-2 text-xs font-medium ${
                                !activeCustomThemeId && themePreference === preference
                                    ? 'border-app-accent/40 bg-app-selected text-app-text'
                                    : 'border-transparent text-app-text-secondary hover:text-app-text'
                            }`}
                        >
                            <Icon className="h-3.5 w-3.5" />
                            {label}
                        </button>
                    ))}
                </div>
                <p className="text-xs leading-relaxed text-app-text-tertiary">
                    Default restores the Quiet Utility theme. System follows your device, while presets and custom themes override these standard modes.
                </p>
            </div>

            <div className="space-y-2">
                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                    <Palette className="w-3.5 h-3.5" />
                    {t('settings.presets')}
                </h3>
                <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
                    {builtinThemes.map(theme => (
                        <button
                            key={theme.id}
                            type="button"
                            aria-pressed={activeCustomThemeId === theme.id}
                            onClick={() => handleSelectTheme(theme)}
                            className={`relative rounded-container border p-1.5 transition-colors ${
                                activeCustomThemeId === theme.id
                                    ? 'border-app-accent bg-app-selected'
                                    : 'border-app-border-subtle hover:border-app-border-strong'
                            }`}
                            title={themeName(theme)}
                        >
                            <div className="flex h-12 overflow-hidden rounded-control">
                                <div className="flex-1" style={{ background: theme.palette.bg }} />
                                <div className="flex-1" style={{ background: theme.palette.surface }} />
                                <div className="flex-1" style={{ background: theme.palette.primary }} />
                            </div>
                            <p className="mt-1.5 truncate text-center text-metadata text-app-text-secondary">
                                {themeName(theme)}
                            </p>
                            {activeCustomThemeId === theme.id && (
                                <div className="absolute -end-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-app-accent">
                                    <Check className="h-2.5 w-2.5 text-app-accent-contrast" />
                                </div>
                            )}
                        </button>
                    ))}
                </div>
            </div>

            <div className="space-y-2">
                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                    <Sparkles className="w-3.5 h-3.5" />
                    {t('settings.custom_themes')}
                </h3>

                {userThemes.length > 0 && (
                    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
                        {userThemes.map(theme => (
                            <button
                                key={theme.id}
                                type="button"
                                aria-pressed={activeCustomThemeId === theme.id}
                                onClick={() => handleSelectTheme(theme)}
                                className={`relative rounded-container border p-1.5 transition-colors ${
                                    activeCustomThemeId === theme.id
                                        ? 'border-app-accent bg-app-selected'
                                        : 'border-app-border-subtle hover:border-app-border-strong'
                                }`}
                                title={themeName(theme)}
                            >
                                <div className="flex h-12 overflow-hidden rounded-control">
                                    <div className="flex-1" style={{ background: theme.palette.bg }} />
                                    <div className="flex-1" style={{ background: theme.palette.surface }} />
                                    <div className="flex-1" style={{ background: theme.palette.primary }} />
                                </div>
                                <p className="mt-1.5 truncate text-center text-metadata text-app-text-secondary">
                                    {themeName(theme)}
                                </p>
                                {activeCustomThemeId === theme.id && (
                                    <div className="absolute -end-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-app-accent">
                                        <Check className="h-2.5 w-2.5 text-app-accent-contrast" />
                                    </div>
                                )}
                            </button>
                        ))}
                    </div>
                )}

                <button
                    onClick={handleCreateTheme}
                    className="quiet-control flex w-full items-center justify-center gap-2 border border-dashed border-app-border px-3 py-2.5 text-xs text-app-text-secondary hover:border-app-accent/50 hover:text-app-accent"
                >
                    <Plus className="w-3.5 h-3.5" />
                    {t('settings.create_theme')}
                </button>
            </div>

            {editingTheme && !editingTheme.isBuiltin && (
                <div className="quiet-surface space-y-4 p-4">
                    <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider">
                        {t('settings.edit_theme')}
                    </h3>

                    <div className="flex items-center gap-2">
                        <label htmlFor="custom-theme-name" className="text-xs text-app-text-secondary w-16 shrink-0">{t('common.name')}</label>
                        <input
                            id="custom-theme-name"
                            type="text"
                            value={editingTheme.name}
                            onChange={event => handleNameChange(event.target.value)}
                            className="quiet-control min-w-0 flex-1 border border-app-border bg-app-surface-sunken px-2.5 py-2 text-ui text-app-text focus:border-app-accent"
                            maxLength={32}
                        />
                    </div>

                    <div className="flex items-center gap-2">
                        <label className="text-xs text-telegram-subtext w-16 shrink-0">{t('settings.base_mode')}</label>
                        <div className="flex gap-1" role="group" aria-label={t('settings.base_mode')}>
                            <button
                                type="button" aria-pressed={editingTheme.isDark}
                                onClick={() => handleBaseToggle(true)}
                                className={`px-3 py-1 rounded-md text-xs font-medium transition ${
                                    editingTheme.isDark
                                        ? 'bg-app-accent text-app-accent-contrast'
                                        : 'bg-telegram-hover text-telegram-subtext hover:text-telegram-text'
                                }`}
                            >
                                {t('common.dark_mode')}
                            </button>
                            <button
                                type="button" aria-pressed={!editingTheme.isDark}
                                onClick={() => handleBaseToggle(false)}
                                className={`px-3 py-1 rounded-md text-xs font-medium transition ${
                                    !editingTheme.isDark
                                        ? 'bg-app-accent text-app-accent-contrast'
                                        : 'bg-telegram-hover text-telegram-subtext hover:text-telegram-text'
                                }`}
                            >
                                {t('common.light_mode')}
                            </button>
                        </div>
                    </div>

                    <div className="space-y-2">
                        {PALETTE_KEYS.map(({ key, labelKey }) => (
                            <div key={key} className="flex items-center gap-2">
                                <label htmlFor={`custom-theme-${key}`} className="w-24 shrink-0 text-xs text-app-text-secondary">{t(labelKey)}</label>
                                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                                    <input
                                        type="color"
                                        aria-label={t(labelKey)}
                                        value={editingTheme.palette[key].startsWith('rgba') ? '#888888' : editingTheme.palette[key]}
                                        onChange={event => handlePaletteChange(key, event.target.value)}
                                        className="w-7 h-7 rounded-md border border-telegram-border cursor-pointer p-0.5 bg-transparent"
                                    />
                                    <input
                                        id={`custom-theme-${key}`}
                                        type="text"
                                        value={editingTheme.palette[key]}
                                        onChange={event => handlePaletteChange(key, event.target.value)}
                                        className="quiet-control min-w-0 flex-1 border border-app-border bg-app-surface-sunken px-2.5 py-2 text-ui text-app-text focus:border-app-accent font-mono"
                                        maxLength={30}
                                    />
                                </div>
                            </div>
                        ))}
                    </div>

                    <button
                        onClick={() => handleDeleteTheme(editingTheme.id)}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-red-400 hover:bg-red-500/10 transition"
                    >
                        <Trash2 className="w-3.5 h-3.5" />
                        {t('settings.delete_theme')}
                    </button>
                </div>
            )}

            {activeCustomThemeId && (
                <button
                    onClick={() => {
                        setThemePreference('default');
                        setEditingId(null);
                    }}
                    className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg text-xs font-medium text-telegram-subtext hover:text-telegram-text bg-telegram-hover/50 hover:bg-telegram-hover transition"
                >
                    <RotateCcw className="w-3.5 h-3.5" />
                    {t('settings.reset_default')}
                </button>
            )}
        </section>
    );
}
