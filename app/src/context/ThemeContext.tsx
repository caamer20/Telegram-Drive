import { createContext, useContext, useState, ReactNode, useLayoutEffect, useCallback } from 'react';
import { CustomTheme, applyTheme as applyThemeToDOM, removeCustomTheme as removeCustomThemeFromDOM } from '../theme/themeEngine';
import { BUILTIN_THEMES } from '../theme/presets';
import { triggerHaptic } from '../services/feedback';
import {
    readActiveCustomTheme,
    readThemePreference,
    readUserThemes,
    writeActiveCustomTheme,
    writeBaseTheme,
    writeUserThemes,
} from '../services/themePersistence';
import type { Theme, ThemePreference } from '../types/settings';

interface ThemeContextType {
    theme: Theme;
    themePreference: ThemePreference;
    toggleTheme: () => void;
    setTheme: (theme: Theme) => void;
    setThemePreference: (theme: ThemePreference) => void;
    // Custom theme engine
    customThemes: CustomTheme[];
    activeCustomThemeId: string | null;
    setActiveCustomTheme: (id: string | null) => void;
    addCustomTheme: (theme: CustomTheme) => void;
    deleteCustomTheme: (id: string) => void;
    updateCustomTheme: (id: string, patch: Partial<CustomTheme>) => void;
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined);

function getSystemTheme(): Theme {
    return typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: light)').matches
        ? 'light'
        : 'dark';
}

function getInitialPreference(): ThemePreference {
    return readThemePreference();
}

function resolveTheme(preference: ThemePreference, systemTheme: Theme): Theme {
    if (preference === 'system') return systemTheme;
    if (preference === 'default') return 'dark';
    return preference;
}

function applyBaseTheme(theme: Theme) {
    const root = document.documentElement;
    if (theme === 'light') {
        root.classList.add('light');
        root.classList.remove('dark');
    } else {
        root.classList.add('dark');
        root.classList.remove('light');
    }
}

if (typeof window !== 'undefined') {
    const initialPreference = getInitialPreference();
    applyBaseTheme(resolveTheme(initialPreference, getSystemTheme()));
}

export function ThemeProvider({ children }: { children: ReactNode }) {
    const [themePreference, setThemePreferenceState] = useState<ThemePreference>(getInitialPreference);
    const [systemTheme, setSystemTheme] = useState<Theme>(getSystemTheme);
    const theme: Theme = resolveTheme(themePreference, systemTheme);
    const [userThemes, setUserThemes] = useState<CustomTheme[]>(() => readUserThemes());
    const [activeCustomThemeId, setActiveCustomThemeIdState] = useState<string | null>(
        () => readActiveCustomTheme()
    );

    const allThemes = [...BUILTIN_THEMES, ...userThemes];

    useLayoutEffect(() => {
        const query = window.matchMedia('(prefers-color-scheme: light)');
        const handleChange = (event: MediaQueryListEvent) => setSystemTheme(event.matches ? 'light' : 'dark');
        query.addEventListener('change', handleChange);
        return () => query.removeEventListener('change', handleChange);
    }, []);

    useLayoutEffect(() => {
        if (!activeCustomThemeId) {
            removeCustomThemeFromDOM();
            applyBaseTheme(theme);
        }
        writeBaseTheme(theme, themePreference);
    }, [theme, themePreference, activeCustomThemeId]);

    useLayoutEffect(() => {
        if (activeCustomThemeId) {
            const found = allThemes.find(t => t.id === activeCustomThemeId);
            if (found) {
                applyThemeToDOM(found);
            } else {
                setActiveCustomThemeIdState(null);
                writeActiveCustomTheme(null);
                removeCustomThemeFromDOM();
                applyBaseTheme(theme);
            }
        }
    }, [activeCustomThemeId, allThemes, theme]);

    const toggleTheme = useCallback(() => {
        triggerHaptic('selection');
        if (activeCustomThemeId) {
            const activeTheme = allThemes.find(t => t.id === activeCustomThemeId);
            const nextBase: Theme = activeTheme?.isDark ? 'light' : 'dark';
            setActiveCustomThemeIdState(null);
            writeActiveCustomTheme(null);
            removeCustomThemeFromDOM();
            setThemePreferenceState(nextBase);
        } else {
            setThemePreferenceState(theme === 'dark' ? 'light' : 'dark');
        }
    }, [activeCustomThemeId, allThemes, theme]);

    const setTheme = useCallback((newTheme: Theme) => {
        setActiveCustomThemeIdState(null);
        writeActiveCustomTheme(null);
        removeCustomThemeFromDOM();
        setThemePreferenceState(newTheme);
    }, []);

    const setThemePreference = useCallback((newTheme: ThemePreference) => {
        setActiveCustomThemeIdState(null);
        writeActiveCustomTheme(null);
        removeCustomThemeFromDOM();
        setThemePreferenceState(newTheme);
    }, []);

    const setActiveCustomTheme = useCallback((id: string | null) => {
        setActiveCustomThemeIdState(id);
        writeActiveCustomTheme(id);
        if (!id) {
            removeCustomThemeFromDOM();
            applyBaseTheme(theme);
        }
    }, [theme]);

    const addCustomTheme = useCallback((t: CustomTheme) => {
        setUserThemes(prev => {
            const next = [...prev, t];
            writeUserThemes(next);
            return next;
        });
    }, []);

    const deleteCustomTheme = useCallback((id: string) => {
        setUserThemes(prev => {
            const next = prev.filter(t => t.id !== id);
            writeUserThemes(next);
            return next;
        });
        setActiveCustomThemeIdState(prev => {
            if (prev === id) {
                writeActiveCustomTheme(null);
                removeCustomThemeFromDOM();
                applyBaseTheme(theme);
                return null;
            }
            return prev;
        });
    }, [theme]);

    const updateCustomTheme = useCallback((id: string, patch: Partial<CustomTheme>) => {
        setUserThemes(prev => {
            const next = prev.map(t => t.id === id ? { ...t, ...patch, id } : t);
            writeUserThemes(next);
            return next;
        });
    }, []);

    return (
        <ThemeContext.Provider value={{
            theme,
            themePreference,
            toggleTheme,
            setTheme,
            setThemePreference,
            customThemes: allThemes,
            activeCustomThemeId,
            setActiveCustomTheme,
            addCustomTheme,
            deleteCustomTheme,
            updateCustomTheme,
        }}>
            {children}
        </ThemeContext.Provider>
    );
}

export const useTheme = () => {
    const context = useContext(ThemeContext);
    if (!context) throw new Error('useTheme must be used within a ThemeProvider');
    return context;
};
