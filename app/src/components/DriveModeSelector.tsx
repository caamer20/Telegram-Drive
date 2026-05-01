import { HardDrive, Lock, ArrowRight } from "lucide-react";
import { DriveMode } from "../types";

export function DriveModeSelector({ onSelect }: { onSelect: (mode: DriveMode) => void }) {
    return (
        <div className="h-full w-full auth-gradient flex items-center justify-center p-6">
            <div className="auth-glass p-8 rounded-3xl shadow-2xl w-full max-w-2xl">
                <div className="text-center mb-8">
                    <div className="w-16 h-16 mb-5 mx-auto flex items-center justify-center">
                        <img src="/logo.svg" alt="Logo" className="w-full h-full" />
                    </div>
                    <h1 className="text-2xl font-bold text-white mb-2 tracking-tight">Choose Storage Mode</h1>
                    <p className="text-sm text-white/60">Use the existing Telegram Drive or open an encrypted vault.</p>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                    <button
                        type="button"
                        onClick={() => onSelect('plain')}
                        className="group rounded-2xl border border-white/10 bg-white/5 p-5 text-left hover:border-blue-400/50 hover:bg-blue-500/10 transition-colors"
                    >
                        <div className="flex items-center justify-between mb-4">
                            <div className="w-12 h-12 rounded-xl bg-blue-500/20 flex items-center justify-center">
                                <HardDrive className="w-6 h-6 text-blue-300" />
                            </div>
                            <ArrowRight className="w-5 h-5 text-white/30 group-hover:text-white" />
                        </div>
                        <h2 className="storage-mode-title text-lg font-semibold text-white mb-2">Normal Drive</h2>
                        <p className="storage-mode-copy text-sm text-white/60 leading-relaxed">
                            Continue with Saved Messages and Telegram channels exactly as the app works today.
                        </p>
                    </button>

                    <button
                        type="button"
                        onClick={() => onSelect('vault')}
                        className="group rounded-2xl border border-white/10 bg-white/5 p-5 text-left hover:border-emerald-400/50 hover:bg-emerald-500/10 transition-colors"
                    >
                        <div className="flex items-center justify-between mb-4">
                            <div className="w-12 h-12 rounded-xl bg-emerald-500/20 flex items-center justify-center">
                                <Lock className="w-6 h-6 text-emerald-300" />
                            </div>
                            <ArrowRight className="w-5 h-5 text-white/30 group-hover:text-white" />
                        </div>
                        <h2 className="storage-mode-title text-lg font-semibold text-white mb-2">Encrypted Vault</h2>
                        <p className="storage-mode-copy text-sm text-white/60 leading-relaxed">
                            Store encrypted blobs in a private TelegramVault channel after unlocking locally.
                        </p>
                    </button>
                </div>
            </div>
        </div>
    );
}
