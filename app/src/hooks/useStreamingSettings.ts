import { useState, useEffect, useCallback } from 'react';
import { StreamingQuality, StreamingSettings } from '../types';

const STORAGE_KEY = 'streaming_settings_v1';

const DEFAULTS: StreamingSettings = {
    quality: 'original',
    adaptiveMode: false,
};

function loadSettings(): StreamingSettings {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (raw) {
            const parsed = JSON.parse(raw);
            if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
                return { ...DEFAULTS };
            }

            // Validate quality
            const validQualities: StreamingQuality[] = ['360p', '480p', '720p', '1080p', 'original'];
            const quality = validQualities.includes(parsed.quality) ? parsed.quality : DEFAULTS.quality;
            const adaptiveMode =
                typeof parsed.adaptiveMode === 'boolean' ? parsed.adaptiveMode : DEFAULTS.adaptiveMode;

            return { quality, adaptiveMode };
        }
    } catch { /* corrupt data, use defaults */ }
    return { ...DEFAULTS };
}

export function useStreamingSettings() {
    const [settings, setSettings] = useState<StreamingSettings>(loadSettings);

    useEffect(() => {
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
        } catch { /* storage full or unavailable */ }
    }, [settings]);

    const setQuality = useCallback((quality: StreamingQuality) => {
        setSettings(prev => ({ ...prev, quality }));
    }, []);

    const setAdaptiveMode = useCallback((adaptiveMode: boolean) => {
        setSettings(prev => ({ ...prev, adaptiveMode }));
    }, []);

    return { settings, setQuality, setAdaptiveMode };
}
