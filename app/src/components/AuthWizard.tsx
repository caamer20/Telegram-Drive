import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion, AnimatePresence } from "framer-motion";
import { Phone, Key, ArrowRight, Settings, ShieldCheck, Sun, Moon, HelpCircle, ExternalLink, X } from "lucide-react";
import { load } from '@tauri-apps/plugin-store';
import { useTheme } from '../context/ThemeContext';

type Step = "setup" | "phone" | "code" | "password";

function AuthThemeToggle() {
    const { theme, toggleTheme } = useTheme();
    return (
        <button
            onClick={toggleTheme}
            className="absolute top-4 right-4 p-3 rounded-full bg-white/10 hover:bg-white/20 transition-colors z-10"
            title={theme === 'dark' ? 'Switch to Light Mode' : 'Switch to Dark Mode'}
        >
            {theme === 'dark' ? (
                <Sun className="w-5 h-5 text-white" />
            ) : (
                <Moon className="w-5 h-5 text-white" />
            )}
        </button>
    );
}
export function AuthWizard({ onLogin }: { onLogin: () => void }) {
    const isBrowser = typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);

    if (isBrowser) {
        return (
            <div className="flex flex-col items-center justify-center h-full max-w-lg mx-auto p-8 text-center">
                <div className="w-20 h-20 bg-red-500/20 rounded-full flex items-center justify-center mb-6">
                    <ShieldCheck className="w-10 h-10 text-red-500" />
                </div>
                <h1 className="text-2xl font-bold text-white mb-4">Desktop App Required</h1>
                <p className="text-gray-400 mb-6 leading-relaxed">
                    You are viewing the internal development server in a browser.
                    This application cannot function here because it requires access to the system backend (Rust).
                </p>
                <div className="p-4 bg-gray-800 rounded-xl border border-gray-700 text-sm text-gray-300">
                    Please open the <strong>Telegram Drive</strong> window in your OS taskbar/dock to continue.
                </div>
            </div>
        )
    }

    const [step, setStep] = useState<Step>("setup");
    const [loading, setLoading] = useState(false);

    const [apiId, setApiId] = useState("");
    const [apiHash, setApiHash] = useState("");

    const [phone, setPhone] = useState("");
    const [code, setCode] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [floodWait, setFloodWait] = useState<number | null>(null);
    const [showHelp, setShowHelp] = useState(false);


    useEffect(() => {
        if (!floodWait) return;
        const interval = setInterval(() => {
            setFloodWait(prev => {
                if (prev === null || prev <= 1) return null;
                return prev - 1;
            });
        }, 1000);
        return () => clearInterval(interval);
    }, [floodWait]);

    useEffect(() => {
        const initStore = async () => {
            try {
                const store = await load('config.json');
                const savedId = await store.get<string>('api_id');
                const savedHash = await store.get<string>('api_hash');

                if (savedId && savedHash) {
                    setApiId(savedId);
                    setApiHash(savedHash);
                }
            } catch {
                // config not found, starting fresh
            }
        };
        initStore();
    }, []);

    const saveCredentials = async () => {
        try {
            const store = await load('config.json');
            await store.set('api_id', apiId);
            await store.set('api_hash', apiHash);
            await store.save();
        } catch {
            // store write failure, non-critical
        }
    };

    const handleSetupSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!apiId || !apiHash) {
            setError("Both API ID and Hash are required.");
            return;
        }
        setError(null);
        await saveCredentials();
        setStep("phone");
    };

    const handlePhoneSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setLoading(true);
        setError(null);
        try {
            const idInt = parseInt(apiId, 10);
            if (isNaN(idInt)) throw new Error("API ID must be a number");

            await invoke("cmd_auth_request_code", {
                phone,
                apiId: idInt,
                apiHash: apiHash
            });
            setStep("code");
        } catch (err: unknown) {
            const msg = err instanceof Error ? err.message : JSON.stringify(err);
            if (msg.includes("FLOOD_WAIT_")) {
                const parts = msg.split("FLOOD_WAIT_");
                if (parts[1]) {
                    const seconds = parseInt(parts[1]);
                    if (!isNaN(seconds)) {
                        setFloodWait(seconds);
                        return;
                    }
                }
            }
            setError(msg);
        } finally {
            setLoading(false);
        }
    };

    const handleCodeSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setLoading(true);
        try {
            const res = await invoke<{ success: boolean; next_step?: string }>("cmd_auth_sign_in", { code });
            if (res.success) {
                onLogin();
            } else if (res.next_step === "password") {
                setStep("password");
            } else {
                setError("Unknown error");
            }
        } catch (err: unknown) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="h-full w-full auth-gradient flex items-center justify-center p-6 relative">
            <AuthThemeToggle />

            <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                className="auth-glass p-8 rounded-3xl shadow-2xl w-full max-w-md"
            >
                <div className="text-center mb-8">
                    <div className="w-20 h-20 mb-6 mx-auto flex items-center justify-center filter drop-shadow-lg">
                        <img src="/logo.svg" alt="Logo" className="w-full h-full" />
                    </div>
                    <h1 className="text-2xl font-bold text-white mb-1 tracking-tight">Telegram Drive</h1>
                    <p className="text-sm text-white/60 font-medium">Self-Hosted Secure Storage</p>
                </div>

                <AnimatePresence mode="wait">
                    {floodWait ? (
                        <motion.div
                            key="flood"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            className="text-center space-y-6"
                        >
                            <div className="w-16 h-16 bg-red-500/20 rounded-full flex items-center justify-center mx-auto animate-pulse">
                                <span className="text-2xl">⏳</span>
                            </div>
                            <div>
                                <h2 className="text-xl font-bold text-white mb-2">Too Many Requests</h2>
                                <p className="text-sm text-gray-400">Telegram has temporarily limited your actions.</p>
                                <p className="text-sm text-gray-400">Please wait before trying again.</p>
                            </div>

                            <div className="text-5xl font-mono items-center justify-center flex text-blue-400 font-bold">
                                {Math.floor(floodWait / 60)}:{(floodWait % 60).toString().padStart(2, '0')}
                            </div>

                            <p className="text-xs text-red-400/60 mt-4">
                                Do not restart the app. The timer will reset if you do.
                            </p>
                        </motion.div>
                    ) : (
                        <>


                            {step === "setup" && (
                                <motion.form
                                    key="setup"
                                    initial={{ x: 20, opacity: 0 }}
                                    animate={{ x: 0, opacity: 1 }}
                                    exit={{ x: -20, opacity: 0 }}
                                    onSubmit={handleSetupSubmit}
                                    className="space-y-5"
                                >
                                    <div className="space-y-4">
                                        <div>
                                            <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">API ID</label>
                                            <div className="relative">
                                                <Key className="absolute left-4 top-1/2 -translate-y-1/2 w-5 h-5 auth-form-icon" />
                                                <input
                                                    type="text"
                                                    value={apiId}
                                                    onChange={(e) => setApiId(e.target.value)}
                                                    placeholder="12345678"
                                                    className="w-full glass-input rounded-xl pl-12 pr-4 py-3.5 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all font-mono text-sm"
                                                />
                                            </div>
                                        </div>
                                        <div>
                                            <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">API Hash</label>
                                            <div className="relative">
                                                <Key className="absolute left-4 top-1/2 -translate-y-1/2 w-5 h-5 auth-form-icon" />
                                                <input
                                                    type="text"
                                                    value={apiHash}
                                                    onChange={(e) => setApiHash(e.target.value)}
                                                    placeholder="abcdef123456..."
                                                    className="w-full glass-input rounded-xl pl-12 pr-4 py-3.5 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all font-mono text-sm"
                                                />
                                            </div>
                                        </div>
                                    </div>

                                    <button
                                        type="submit"
                                        className="w-full bg-gradient-to-r from-blue-600 to-blue-500 hover:from-blue-500 hover:to-blue-400 text-white font-bold py-4 rounded-xl flex items-center justify-center gap-2 transition-all shadow-lg shadow-blue-900/20 active:scale-[0.98]"
                                    >
                                        Configure <Settings className="w-4 h-4" />
                                    </button>

                                    <button
                                        type="button"
                                        onClick={() => setShowHelp(true)}
                                        className="w-full text-xs text-blue-300 hover:text-white transition-colors flex items-center justify-center gap-1.5 py-1"
                                    >
                                        <HelpCircle className="w-3 h-3" />
                                        How do I get my API credentials?
                                    </button>

                                    <button
                                        type="button"
                                        onClick={() => onLogin()}
                                        className="w-full text-xs text-red-400/60 hover:text-red-300 transition-colors py-1"
                                    >
                                        Dev Mode
                                    </button>
                                </motion.form>
                            )}


                            {step === "phone" && (
                                <motion.form
                                    key="phone"
                                    initial={{ x: 20, opacity: 0 }}
                                    animate={{ x: 0, opacity: 1 }}
                                    exit={{ x: -20, opacity: 0 }}
                                    onSubmit={handlePhoneSubmit}
                                    className="space-y-6"
                                >
                                    <div className="space-y-2">
                                        <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider">Phone Number</label>
                                        <div className="relative">
                                            <Phone className="absolute left-4 top-1/2 -translate-y-1/2 w-5 h-5 auth-form-icon" />
                                            <input
                                                type="tel"
                                                value={phone}
                                                onChange={(e) => setPhone(e.target.value)}
                                                placeholder="+1 234 567 8900"
                                                className="w-full glass-input rounded-xl pl-12 pr-4 py-4 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all text-lg tracking-wide"
                                            />
                                        </div>
                                    </div>

                                    <div className="flex flex-col gap-3">
                                        <button
                                            type="submit"
                                            disabled={loading}
                                            className="w-full bg-white text-black hover:bg-gray-100 font-bold py-4 rounded-xl flex items-center justify-center gap-2 transition-all shadow-lg active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed"
                                        >
                                            {loading ? "Connecting..." : <>Continue <ArrowRight className="w-5 h-5" /></>}
                                        </button>
                                        <button type="button" onClick={() => setStep("setup")} className="text-xs text-gray-500 hover:text-white transition-colors py-2">
                                            Back to Configuration
                                        </button>
                                    </div>
                                </motion.form>
                            )}


                            {step === "code" && (
                                <motion.form
                                    key="code"
                                    initial={{ x: 20, opacity: 0 }}
                                    animate={{ x: 0, opacity: 1 }}
                                    exit={{ x: -20, opacity: 0 }}
                                    onSubmit={handleCodeSubmit}
                                    className="space-y-6"
                                >
                                    <div className="space-y-2">
                                        <label className="block text-xs font-semibold text-gray-400 uppercase tracking-wider">Telegram Code</label>
                                        <div className="relative">
                                            <Key className="absolute left-4 top-1/2 -translate-y-1/2 w-5 h-5 auth-form-icon" />
                                            <input
                                                type="text"
                                                value={code}
                                                onChange={(e) => setCode(e.target.value)}
                                                placeholder="1 2 3 4 5"
                                                className="w-full glass-input rounded-xl pl-12 pr-4 py-4 text-white placeholder-gray-600 focus:outline-none focus:border-blue-500 transition-all text-2xl tracking-[0.5em] font-mono text-center"
                                            />
                                        </div>
                                    </div>

                                    <div className="flex flex-col gap-3">
                                        <button
                                            type="submit"
                                            disabled={loading}
                                            className="w-full bg-white text-black hover:bg-gray-100 font-bold py-4 rounded-xl flex items-center justify-center gap-2 transition-all shadow-lg active:scale-[0.98]"
                                        >
                                            {loading ? "Verifying..." : "Sign In"}
                                        </button>
                                        <button type="button" onClick={() => setStep("phone")} className="text-xs text-gray-500 hover:text-white transition-colors py-2">
                                            Change Phone Number
                                        </button>
                                    </div>
                                </motion.form>
                            )}
                        </>
                    )}
                </AnimatePresence>

                {error && (
                    <motion.div
                        initial={{ opacity: 0, y: 10 }}
                        animate={{ opacity: 1, y: 0 }}
                        className="mt-6 p-4 bg-red-500/10 border border-red-500/20 rounded-xl flex items-start gap-3"
                    >
                        <div className="w-1.5 h-1.5 rounded-full bg-red-500 mt-2 shrink-0" />
                        <p className="text-red-400 text-sm leading-snug">{error}</p>
                    </motion.div>
                )}
            </motion.div>


            <AnimatePresence>
                {showHelp && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4"
                        onClick={() => setShowHelp(false)}
                    >
                        <motion.div
                            initial={{ scale: 0.95, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.95, opacity: 0 }}
                            className="glass bg-telegram-surface border border-telegram-border rounded-2xl p-6 max-w-lg w-full max-h-[80vh] overflow-y-auto shadow-2xl"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <div className="flex items-center justify-between mb-6">
                                <h2 className="text-xl font-bold text-telegram-text">Getting Started</h2>
                                <button onClick={() => setShowHelp(false)} className="p-2 hover:bg-telegram-hover rounded-lg transition-colors">
                                    <X className="w-5 h-5 text-telegram-subtext" />
                                </button>
                            </div>

                            <div className="space-y-6 text-telegram-text">
                                <div className="p-4 bg-telegram-primary/10 border border-telegram-primary/20 rounded-xl">
                                    <p className="text-sm text-telegram-subtext">
                                        <strong className="text-telegram-primary">Telegram Drive</strong> uses your Telegram account as secure cloud storage. You'll need a Telegram account and API credentials to get started.
                                    </p>
                                </div>

                                <div className="space-y-4">
                                    <h3 className="font-semibold flex items-center gap-2">
                                        <span className="w-6 h-6 bg-telegram-primary text-white text-xs font-bold rounded-full flex items-center justify-center">1</span>
                                        Go to Telegram's Developer Portal
                                    </h3>
                                    <p className="text-sm text-telegram-subtext ml-8">
                                        Visit <a href="https://my.telegram.org" target="_blank" className="text-telegram-primary underline hover:text-telegram-text">my.telegram.org</a> and log in with your phone number.
                                    </p>
                                </div>

                                <div className="space-y-4">
                                    <h3 className="font-semibold flex items-center gap-2">
                                        <span className="w-6 h-6 bg-telegram-primary text-white text-xs font-bold rounded-full flex items-center justify-center">2</span>
                                        Create a New Application
                                    </h3>
                                    <p className="text-sm text-telegram-subtext ml-8">
                                        Click on <strong>"API development tools"</strong> and create a new application. Use any name and description you like.
                                    </p>
                                </div>

                                <div className="space-y-4">
                                    <h3 className="font-semibold flex items-center gap-2">
                                        <span className="w-6 h-6 bg-telegram-primary text-white text-xs font-bold rounded-full flex items-center justify-center">3</span>
                                        Copy Your Credentials
                                    </h3>
                                    <p className="text-sm text-telegram-subtext ml-8">
                                        After creating the app, you'll see your <strong>API ID</strong> (a number) and <strong>API Hash</strong> (a string). Copy both and paste them into the fields on the previous screen.
                                    </p>
                                </div>

                                <div className="p-4 bg-telegram-hover rounded-xl border border-telegram-border">
                                    <p className="text-xs text-telegram-subtext">
                                        <strong>🔒 Privacy:</strong> Your credentials are stored locally on your device and are never sent to any third-party servers. All data goes directly between you and Telegram.
                                    </p>
                                </div>

                                <a
                                    href="https://my.telegram.org"
                                    target="_blank"
                                    className="w-full bg-telegram-primary text-white font-semibold py-3 rounded-xl flex items-center justify-center gap-2 hover:bg-telegram-primary/90 transition-colors"
                                >
                                    <ExternalLink className="w-4 h-4" />
                                    Open my.telegram.org
                                </a>
                            </div>
                        </motion.div>
                    </motion.div>
                )}
            </AnimatePresence>


            <div className="fixed top-[-20%] left-[-10%] w-[500px] h-[500px] bg-blue-600/20 rounded-full blur-[120px] pointer-events-none -z-10" />
            <div className="fixed bottom-[-10%] right-[-10%] w-[400px] h-[400px] bg-purple-600/10 rounded-full blur-[100px] pointer-events-none -z-10" />
        </div>
    );
}
