import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, Lock, LogOut, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

interface VaultStatus {
    configured: boolean;
    unlocked: boolean;
    vaultId?: string;
    generation?: number;
}

export function VaultWizard({ onUnlock, onBack }: { onUnlock: () => void; onBack: () => void }) {
    const [status, setStatus] = useState<VaultStatus | null>(null);
    const [password, setPassword] = useState("");
    const [confirmPassword, setConfirmPassword] = useState("");
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        invoke<VaultStatus>("cmd_vault_status")
            .then((result) => {
                setStatus(result);
                if (result.unlocked) onUnlock();
            })
            .catch((err) => setError(String(err)));
    }, [onUnlock]);

    const configured = status?.configured ?? false;

    const submit = async (e: React.FormEvent) => {
        e.preventDefault();
        setError(null);

        if (!password) {
            setError("Enter your vault password.");
            return;
        }
        if (!configured && password.length < 10) {
            setError("Use at least 10 characters for the vault password.");
            return;
        }
        if (!configured && password !== confirmPassword) {
            setError("Vault passwords do not match.");
            return;
        }

        setLoading(true);
        try {
            const command = configured ? "cmd_vault_unlock" : "cmd_vault_create";
            await invoke<VaultStatus>(command, { password });
            toast.success(configured ? "Vault unlocked" : "Encrypted vault created");
            onUnlock();
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="h-full w-full auth-gradient flex items-center justify-center p-6">
            <form onSubmit={submit} className="auth-glass p-8 rounded-3xl shadow-2xl w-full max-w-md">
                <div className="text-center mb-8">
                    <div className="w-16 h-16 mx-auto mb-5 rounded-2xl bg-telegram-primary/20 flex items-center justify-center">
                        {configured ? <Lock className="w-8 h-8 text-telegram-primary" /> : <ShieldCheck className="w-8 h-8 text-telegram-primary" />}
                    </div>
                    <h1 className="text-2xl font-bold text-white mb-2 tracking-tight">
                        {configured ? "Unlock Encrypted Vault" : "Create Encrypted Vault"}
                    </h1>
                    <p className="text-sm text-white/60">
                        {configured
                            ? "Your Telegram objects stay encrypted until this device unlocks the vault."
                            : "Telegram will store only ciphertext blobs and encrypted vault metadata."}
                    </p>
                </div>

                <div className="space-y-4">
                    <div>
                        <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">Vault Password</label>
                        <input
                            autoFocus
                            type="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            className="w-full glass-input rounded-xl px-4 py-3.5 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all"
                            placeholder="Enter vault password"
                        />
                    </div>

                    {!configured && (
                        <div>
                            <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">Confirm Password</label>
                            <input
                                type="password"
                                value={confirmPassword}
                                onChange={(e) => setConfirmPassword(e.target.value)}
                                className="w-full glass-input rounded-xl px-4 py-3.5 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all"
                                placeholder="Repeat vault password"
                            />
                        </div>
                    )}

                    {error && (
                        <div className="p-3 rounded-xl bg-red-500/10 border border-red-500/20 text-red-300 text-sm">
                            {error}
                        </div>
                    )}

                    <button
                        type="submit"
                        disabled={loading}
                        className="w-full bg-telegram-primary hover:bg-telegram-primary/90 disabled:opacity-60 text-white font-semibold py-3.5 rounded-xl transition-all flex items-center justify-center gap-2"
                    >
                        {loading ? "Working..." : configured ? "Unlock Vault" : "Create Vault"}
                        {!loading && <ArrowRight className="w-4 h-4" />}
                    </button>

                    <button
                        type="button"
                        onClick={onBack}
                        className="w-full text-gray-400 hover:text-white text-sm flex items-center justify-center gap-2 py-2"
                    >
                        <LogOut className="w-4 h-4" />
                        Back to storage modes
                    </button>
                </div>
            </form>
        </div>
    );
}
