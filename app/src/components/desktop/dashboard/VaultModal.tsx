import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
    ShieldCheck, ShieldOff, Lock, Unlock, KeyRound, Folder, FileLock2, Loader2, X, Settings2,
} from 'lucide-react';
import { useEncryption } from '../../../hooks/useEncryption';
import { formatBytes } from '../../../utils';

interface VaultFileEntry {
    folder_id: number | null;
    folder_name: string | null;
    message_id: number;
    file_name: string;
    file_size: number;
    created_at: string;
    encryption_state: string;
}

interface VaultModalProps {
    onClose: () => void;
    onOpenFolder: (folderId: number | null) => void;
    onOpenEncryptionSettings: () => void;
}

/**
 * The vault in one place: create it, unlock/lock it, and see what is protected.
 * Files never move into a separate container — the vault encrypts in place, so
 * an entry here still lives in its folder.
 */
export function VaultModal({ onClose, onOpenFolder, onOpenEncryptionSettings }: VaultModalProps) {
    const encryption = useEncryption();
    const vault = encryption.vaultStatus;
    const exists = vault?.exists ?? false;
    const unlocked = vault?.is_unlocked ?? false;

    const [passphrase, setPassphrase] = useState('');
    const [confirmPassphrase, setConfirmPassphrase] = useState('');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [files, setFiles] = useState<VaultFileEntry[]>([]);
    const [loadingFiles, setLoadingFiles] = useState(false);

    const loadFiles = useCallback(async () => {
        setLoadingFiles(true);
        try {
            setFiles(await invoke<VaultFileEntry[]>('cmd_list_vault_files'));
        } catch {
            setFiles([]);
        } finally {
            setLoadingFiles(false);
        }
    }, []);

    useEffect(() => { void loadFiles(); }, [loadFiles, unlocked]);

    const run = async (action: () => Promise<unknown>) => {
        setBusy(true);
        setError(null);
        try {
            await action();
            setPassphrase('');
            setConfirmPassphrase('');
            await encryption.refreshVaultStatus();
            await encryption.refreshInventory();
            await loadFiles();
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    const handleCreate = () => {
        if (passphrase.length < 8) { setError('Passphrase must be at least 8 characters.'); return; }
        if (passphrase !== confirmPassphrase) { setError('Passphrases do not match.'); return; }
        void run(() => encryption.createVault(passphrase));
    };

    const handleUnlock = () => {
        if (!passphrase) { setError('Enter your vault passphrase.'); return; }
        void run(() => encryption.unlockVault(passphrase));
    };

    const totalBytes = files.reduce((sum, file) => sum + file.file_size, 0);

    return (
        <div className="fixed inset-0 z-[90] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm" onClick={onClose}>
            <div
                role="dialog"
                aria-modal="true"
                aria-label="Vault"
                className="flex max-h-[85vh] w-[560px] flex-col overflow-hidden rounded-2xl border border-app-border bg-app-surface shadow-2xl"
                onClick={e => e.stopPropagation()}
            >
                <header className="flex items-center justify-between border-b border-app-border-subtle px-4 py-3">
                    <div className="flex items-center gap-2">
                        <ShieldCheck className={`h-5 w-5 ${unlocked ? 'text-emerald-400' : 'text-fuchsia-400'}`} />
                        <div>
                            <h2 className="text-ui font-semibold text-app-text">Vault</h2>
                            <p className="text-badge text-app-text-secondary">
                                {!exists ? 'Not created yet' : unlocked ? 'Unlocked — protected files can open' : 'Locked'}
                            </p>
                        </div>
                    </div>
                    <button type="button" onClick={onClose} aria-label="Close" className="quiet-control p-1.5 text-app-text-secondary hover:bg-app-hover">
                        <X className="h-4 w-4" />
                    </button>
                </header>

                <div className="flex-1 space-y-4 overflow-y-auto p-4">
                    {!exists && (
                        <section className="space-y-2 rounded-xl border border-app-border bg-app-surface-sunken/40 p-3">
                            <p className="text-metadata text-app-text-secondary">
                                Create a vault passphrase. It encrypts files you mark as protected and is never stored —
                                keep a recovery key. Minimum 8 characters.
                            </p>
                            <input
                                type="password"
                                value={passphrase}
                                onChange={e => setPassphrase(e.target.value)}
                                placeholder="New vault passphrase"
                                className="quiet-control h-9 w-full rounded-lg border border-app-border bg-app-surface px-3 text-ui"
                            />
                            <input
                                type="password"
                                value={confirmPassphrase}
                                onChange={e => setConfirmPassphrase(e.target.value)}
                                placeholder="Confirm passphrase"
                                className="quiet-control h-9 w-full rounded-lg border border-app-border bg-app-surface px-3 text-ui"
                            />
                            <button
                                type="button"
                                disabled={busy}
                                onClick={handleCreate}
                                className="quiet-control flex h-9 w-full items-center justify-center gap-2 rounded-lg bg-app-accent px-3 text-ui font-medium text-white disabled:opacity-50"
                            >
                                {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <KeyRound className="h-4 w-4" />}
                                Create vault
                            </button>
                        </section>
                    )}

                    {exists && !unlocked && (
                        <section className="space-y-2 rounded-xl border border-app-border bg-app-surface-sunken/40 p-3">
                            <p className="text-metadata text-app-text-secondary">Unlock to open protected files.</p>
                            <input
                                type="password"
                                value={passphrase}
                                onChange={e => setPassphrase(e.target.value)}
                                placeholder="Vault passphrase"
                                className="quiet-control h-9 w-full rounded-lg border border-app-border bg-app-surface px-3 text-ui"
                            />
                            <button
                                type="button"
                                disabled={busy}
                                onClick={handleUnlock}
                                className="quiet-control flex h-9 w-full items-center justify-center gap-2 rounded-lg bg-app-accent px-3 text-ui font-medium text-white disabled:opacity-50"
                            >
                                {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Unlock className="h-4 w-4" />}
                                Unlock vault
                            </button>
                        </section>
                    )}

                    {exists && unlocked && (
                        <section className="flex items-center justify-between rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-3 py-2">
                            <span className="flex items-center gap-2 text-metadata text-emerald-300">
                                <Unlock className="h-4 w-4" />Vault unlocked
                            </span>
                            <button
                                type="button"
                                disabled={busy}
                                onClick={() => void run(() => encryption.lockVault())}
                                className="quiet-control flex h-8 items-center gap-1.5 rounded-lg border border-app-border px-2.5 text-badge font-medium text-app-text hover:bg-app-hover"
                            >
                                <Lock className="h-3.5 w-3.5" />Lock now
                            </button>
                        </section>
                    )}

                    {error && <p className="rounded-lg bg-red-500/10 px-3 py-2 text-badge text-red-400">{error}</p>}

                    <section className="space-y-2">
                        <div className="flex items-center justify-between">
                            <h3 className="text-ui font-semibold text-app-text">Protected files</h3>
                            <span className="text-badge text-app-text-tertiary">
                                {files.length} file{files.length === 1 ? '' : 's'} · {formatBytes(totalBytes)}
                            </span>
                        </div>
                        {loadingFiles ? (
                            <p className="flex items-center gap-2 px-1 py-4 text-metadata text-app-text-secondary">
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />Loading…
                            </p>
                        ) : files.length === 0 ? (
                            <p className="rounded-lg border border-dashed border-app-border px-3 py-6 text-center text-metadata text-app-text-secondary">
                                Nothing is protected yet. Set <strong>Default mode → Vault</strong> in Settings → Encryption,
                                or tick “Protect this upload” in the upload dialog, then upload files. They stay in their folder
                                and show a lock badge.
                            </p>
                        ) : (
                            <ul className="divide-y divide-app-border-subtle overflow-hidden rounded-lg border border-app-border">
                                {files.map(file => (
                                    <li key={`${file.folder_id ?? 'home'}:${file.message_id}`}>
                                        <button
                                            type="button"
                                            onClick={() => { onOpenFolder(file.folder_id ?? null); onClose(); }}
                                            className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-app-hover"
                                        >
                                            <FileLock2 className="h-4 w-4 shrink-0 text-fuchsia-400" />
                                            <span className="min-w-0 flex-1 truncate text-ui text-app-text">{file.file_name}</span>
                                            <span className="hidden shrink-0 items-center gap-1 text-badge text-app-text-tertiary sm:flex">
                                                <Folder className="h-3 w-3" />
                                                {file.folder_name ?? 'Saved Messages'}
                                            </span>
                                            <span className="shrink-0 text-badge tabular-nums text-app-text-secondary">{formatBytes(file.file_size)}</span>
                                        </button>
                                    </li>
                                ))}
                            </ul>
                        )}
                    </section>

                    <section className="rounded-xl border border-app-border bg-app-surface-sunken/30 p-3 text-metadata text-app-text-secondary">
                        <p className="mb-1 flex items-center gap-1.5 font-medium text-app-text">
                            <ShieldOff className="h-3.5 w-3.5" />How files get in
                        </p>
                        <ol className="list-inside list-decimal space-y-0.5">
                            <li>Open Settings → Encryption and set <strong>Default mode: Vault</strong>, then upload — new uploads are encrypted.</li>
                            <li><strong>Already uploaded:</strong> select the file(s) in their folder and press <strong>Encrypt</strong> in the selection bar (it re-uploads them through the vault and removes the plaintext copy).</li>
                            <li>They stay in their normal folder with a lock badge; this list collects them.</li>
                        </ol>
                    </section>
                </div>

                <footer className="flex items-center justify-between border-t border-app-border-subtle px-4 py-2.5">
                    <button
                        type="button"
                        onClick={() => { onClose(); onOpenEncryptionSettings(); }}
                        className="quiet-control flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-badge font-medium text-app-accent hover:bg-app-hover"
                    >
                        <Settings2 className="h-3.5 w-3.5" />Encryption & recovery settings
                    </button>
                    <span className="text-badge text-app-text-tertiary">Local vault · never leaves device</span>
                </footer>
            </div>
        </div>
    );
}
