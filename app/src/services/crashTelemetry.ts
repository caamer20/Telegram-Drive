import { version as appVersion } from '../../package.json';
import { invoke } from '@tauri-apps/api/core';

type CrashSource = 'react' | 'window' | 'promise';

interface CrashEnvelope {
    event: 'app_crash';
    appVersion: string;
    source: CrashSource;
    errorType: string;
    frames: string[];
    platform: string;
    occurredAt: string;
}

const QUEUE_KEY = 'telegram-drive-crash-reports';
let enabled = false;
let consentId: string | undefined;

function safeFrames(error: unknown): string[] {
    if (!(error instanceof Error) || !error.stack) return [];
    return error.stack
        .split('\n')
        .slice(1, 7)
        .map((line) => line.match(/\bat\s+([A-Za-z0-9_$<>.]+)/)?.[1] || '')
        .filter(Boolean);
}

function readQueue(): CrashEnvelope[] {
    try {
        const parsed = JSON.parse(localStorage.getItem(QUEUE_KEY) || '[]');
        return Array.isArray(parsed) ? parsed.slice(-10) : [];
    } catch {
        return [];
    }
}

function writeQueue(queue: CrashEnvelope[]) {
    try {
        localStorage.setItem(QUEUE_KEY, JSON.stringify(queue.slice(-10)));
    } catch {
        // Storage is best-effort; a reporting failure must never affect the app.
    }
}

async function deliver(envelope: CrashEnvelope) {
    const grant = consentId;
    try {
        await invoke('cmd_submit_crash_report', { report: envelope, consentId: grant });
    } catch {
        if (enabled && consentId === grant) writeQueue([...readQueue(), envelope]);
    }
}

export function configureCrashTelemetry(isEnabled: boolean, grant?: string) {
    enabled = isEnabled;
    consentId = grant;
    if (!enabled) {
        writeQueue([]);
        return;
    }
    const queued = readQueue();
    if (queued.length === 0) return;
    writeQueue([]);
    void Promise.all(queued.map(deliver));
}

/**
 * Reports crash shape only. Error messages, user-entered values, file names,
 * file paths, file contents, Telegram IDs, and stack source paths are omitted.
 */
export function reportCrash(error: unknown, source: CrashSource) {
    if (!enabled) return;
    const envelope: CrashEnvelope = {
        event: 'app_crash',
        appVersion,
        source,
        errorType: error instanceof Error ? error.name : 'UnknownError',
        frames: safeFrames(error),
        platform: navigator.platform || 'unknown',
        occurredAt: new Date().toISOString(),
    };
    void deliver(envelope);
}
