import React, { useState, useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useMigrationContext } from '../../context/MigrationContext';
import { Cloud, Play, Square, FolderOpen, RefreshCw, CheckCircle, UploadCloud, Database } from 'lucide-react';

interface MigrationStatus {
    job_id: number;
    is_running: boolean;
    total_items: number;
    item_stats: Record<string, number>;
    total_folders: number;
    folder_stats: Record<string, number>;
}

export const BackupV2Page: React.FC = () => {
    const {
        msAccount,
        connectMicrosoft,
        switchMicrosoftAccount,
        listOneDriveFolders,
        loading,
    } = useMigrationContext();

    // ---- Configuration state ----
    const [sourceFolderId, setSourceFolderId] = useState<string>('root');
    const [sourceFolderPath, setSourceFolderPath] = useState<string>('/');
    const [localBackupDir, setLocalBackupDir] = useState<string>('');
    const [telegramDestId, setTelegramDestId] = useState<string>(''); // empty means Saved Messages
    const [folders, setFolders] = useState<Array<{ id: string; name: string; path: string }>>([]);
    const [foldersLoading, setFoldersLoading] = useState(false);

    // ---- Job state ----
    const [jobId, setJobId] = useState<number | null>(null);
    const [status, setStatus] = useState<MigrationStatus | null>(null);
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    const loadFolders = useCallback(async (parentId?: string) => {
        setFoldersLoading(true);
        try {
            const items = await listOneDriveFolders(parentId);
            setFolders(items.filter(f => f.item_type === 'folder').map(f => ({
                id: f.id,
                name: f.name,
                path: f.path || f.name,
            })));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setFoldersLoading(false);
        }
    }, [listOneDriveFolders]);

    useEffect(() => {
        if (msAccount) {
            void loadFolders();
        }
    }, [msAccount, loadFolders]);

    const startPolling = useCallback((jid: number) => {
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = setInterval(async () => {
            try {
                const s = await invoke<MigrationStatus>('cmd_migration_get_status', { jobId: jid });
                setStatus(s);
                if (!s.is_running && s.job_id === jid && s.item_stats['failed'] === 0 && s.folder_stats['pending'] === 0 && s.folder_stats['fetching'] === 0) {
                    const completed = (s.item_stats['completed_telegram'] || 0) + (s.item_stats['completed_local'] || 0);
                    if (s.total_items > 0 && completed >= s.total_items) {
                        toast.success('Migration completed successfully!');
                        if (pollRef.current) clearInterval(pollRef.current);
                    }
                }
            } catch {
                // ignore
            }
        }, 1500);
    }, []);

    useEffect(() => {
        return () => {
            if (pollRef.current) clearInterval(pollRef.current);
        };
    }, []);

    const handleStart = async () => {
        if (!sourceFolderId) return toast.error('Please select a source folder');
        if (!localBackupDir) return toast.error('Please specify a local backup directory');
        
        const destIdNum = telegramDestId ? parseInt(telegramDestId, 10) : null;
        if (telegramDestId && isNaN(destIdNum!)) return toast.error('Invalid Telegram Destination ID');

        try {
            const newJobId = await invoke<number>('cmd_migration_start', {
                sourceFolderId,
                sourceFolderPath: sourceFolderPath || '/',
                telegramDestinationId: destIdNum,
                telegramDestinationName: telegramDestId ? "Custom Chat" : "Saved Messages",
                localBackupDir,
            });
            setJobId(newJobId);
            toast.success('Migration started!');
            startPolling(newJobId);
        } catch (e: any) {
            toast.error(String(e));
        }
    };

    const handleStop = async () => {
        try {
            await invoke('cmd_migration_stop');
            toast.info('Migration stopped');
        } catch (e: any) {
            toast.error(String(e));
        }
    };
    
    const handleRetry = async () => {
        if (!jobId) return;
        try {
            await invoke('cmd_migration_retry_failed', { jobId });
            toast.success('Retrying failed items...');
        } catch (e: any) {
            toast.error(String(e));
        }
    };

    const handleResetDB = async () => {
        if (!window.confirm("Are you sure you want to reset the migration database? This will clear all history and queues.")) return;
        try {
            await invoke('cmd_migration_reset_database');
            setJobId(null);
            setStatus(null);
            toast.success('Database reset successfully.');
        } catch (e: any) {
            toast.error(String(e));
        }
    };

    if (loading) return <div className="flex h-full items-center justify-center"><RefreshCw className="animate-spin text-telegram-primary w-8 h-8" /></div>;

    if (!msAccount) {
        return (
            <div className="flex flex-col items-center justify-center h-full p-8 text-center bg-telegram-bg">
                <Cloud className="w-16 h-16 text-telegram-primary mb-4" />
                <h2 className="text-2xl font-bold text-telegram-text mb-2">Connect OneDrive</h2>
                <p className="text-telegram-subtext mb-6 max-w-md">
                    Connect your Microsoft account to start migrating your files to Telegram safely and efficiently.
                </p>
                <button
                    onClick={() => connectMicrosoft()}
                    className="bg-telegram-primary hover:bg-telegram-primary/90 text-white px-8 py-3 rounded-xl font-medium transition-colors"
                >
                    Connect Microsoft Account
                </button>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full bg-telegram-bg overflow-y-auto">
            <div className="p-6 border-b border-telegram-divider bg-telegram-card sticky top-0 z-10 flex items-center justify-between">
                <div>
                    <h1 className="text-2xl font-bold text-telegram-text flex items-center gap-2">
                        <Cloud className="text-telegram-primary" />
                        OneDrive Migration
                    </h1>
                    <p className="text-sm text-telegram-subtext mt-1">
                        Connected as <span className="font-medium text-telegram-text">{msAccount.account_name}</span> ({msAccount.account_email})
                    </p>
                </div>
                <div className="flex gap-2">
                    <button onClick={switchMicrosoftAccount} className="px-4 py-2 text-sm bg-telegram-hover text-telegram-text rounded-lg hover:bg-telegram-divider transition-colors">
                        Switch Account
                    </button>
                    <button onClick={handleResetDB} className="px-4 py-2 text-sm bg-red-500/10 text-red-500 rounded-lg hover:bg-red-500/20 transition-colors flex items-center gap-1">
                        <Database className="w-4 h-4" /> Reset DB
                    </button>
                </div>
            </div>

            <div className="p-6 max-w-5xl mx-auto w-full space-y-6">
                
                {/* Configuration Panel */}
                <div className="bg-telegram-card rounded-2xl border border-telegram-divider p-6 space-y-5">
                    <h2 className="text-lg font-semibold text-telegram-text flex items-center gap-2">
                        <FolderOpen className="w-5 h-5 text-telegram-primary" />
                        Migration Settings
                    </h2>
                    
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <div className="space-y-2">
                            <label className="text-sm font-medium text-telegram-subtext">Source Folder (OneDrive)</label>
                            <select 
                                value={sourceFolderId}
                                onChange={(e) => {
                                    setSourceFolderId(e.target.value);
                                    const f = folders.find(x => x.id === e.target.value);
                                    if (f) setSourceFolderPath(f.path);
                                    else setSourceFolderPath('/');
                                }}
                                disabled={foldersLoading || status?.is_running}
                                className="w-full bg-telegram-bg border border-telegram-divider rounded-lg px-4 py-2 text-telegram-text focus:outline-none focus:border-telegram-primary transition-colors"
                            >
                                <option value="root">Root Directory (/)</option>
                                {folders.map(f => (
                                    <option key={f.id} value={f.id}>{f.path} ({f.name})</option>
                                ))}
                            </select>
                        </div>

                        <div className="space-y-2">
                            <label className="text-sm font-medium text-telegram-subtext">Local Backup Directory (Required)</label>
                            <input 
                                type="text"
                                value={localBackupDir}
                                onChange={(e) => setLocalBackupDir(e.target.value)}
                                disabled={status?.is_running}
                                placeholder="/Volumes/Backup/Telegram"
                                className="w-full bg-telegram-bg border border-telegram-divider rounded-lg px-4 py-2 text-telegram-text focus:outline-none focus:border-telegram-primary transition-colors placeholder-telegram-subtext/50"
                            />
                        </div>
                        
                        <div className="space-y-2">
                            <label className="text-sm font-medium text-telegram-subtext">Telegram Destination ID (Optional)</label>
                            <input 
                                type="text"
                                value={telegramDestId}
                                onChange={(e) => setTelegramDestId(e.target.value)}
                                disabled={status?.is_running}
                                placeholder="Leave empty for Saved Messages"
                                className="w-full bg-telegram-bg border border-telegram-divider rounded-lg px-4 py-2 text-telegram-text focus:outline-none focus:border-telegram-primary transition-colors placeholder-telegram-subtext/50"
                            />
                        </div>
                    </div>

                    <div className="pt-4 flex gap-3">
                        {!status?.is_running ? (
                            <button
                                onClick={handleStart}
                                className="flex-1 bg-telegram-primary hover:bg-telegram-primary/90 text-white font-medium py-3 px-6 rounded-xl transition-all active:scale-[0.98] flex items-center justify-center gap-2 shadow-lg shadow-telegram-primary/20"
                            >
                                <Play className="w-5 h-5 fill-current" />
                                Start Migration
                            </button>
                        ) : (
                            <button
                                onClick={handleStop}
                                className="flex-1 bg-red-500 hover:bg-red-600 text-white font-medium py-3 px-6 rounded-xl transition-all active:scale-[0.98] flex items-center justify-center gap-2 shadow-lg shadow-red-500/20"
                            >
                                <Square className="w-5 h-5 fill-current" />
                                Stop Migration
                            </button>
                        )}
                        
                        {status && (status.item_stats['failed'] || 0) > 0 && !status.is_running && (
                            <button
                                onClick={handleRetry}
                                className="bg-orange-500 hover:bg-orange-600 text-white font-medium py-3 px-6 rounded-xl transition-all active:scale-[0.98] flex items-center justify-center gap-2 shadow-lg shadow-orange-500/20"
                            >
                                <RefreshCw className="w-5 h-5" />
                                Retry Failed
                            </button>
                        )}
                    </div>
                </div>

                {/* Status Dashboard */}
                {status && (
                    <div className="bg-telegram-card rounded-2xl border border-telegram-divider p-6 space-y-6 shadow-xl shadow-telegram-primary/5">
                        <div className="flex items-center justify-between">
                            <h2 className="text-lg font-semibold text-telegram-text flex items-center gap-2">
                                <UploadCloud className="w-5 h-5 text-telegram-primary" />
                                Live Progress
                            </h2>
                            <div className="flex items-center gap-2">
                                <div className={`w-2.5 h-2.5 rounded-full ${status.is_running ? 'bg-green-500 animate-pulse' : 'bg-gray-500'}`} />
                                <span className="text-sm font-medium text-telegram-subtext uppercase tracking-wider">
                                    {status.is_running ? 'Running' : 'Stopped'}
                                </span>
                            </div>
                        </div>

                        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                            <StatCard 
                                title="Folders Discovered" 
                                value={status.total_folders} 
                                subValue={`${status.folder_stats['completed'] || 0} completed`}
                                icon={<FolderOpen className="text-blue-500 w-6 h-6" />}
                            />
                            <StatCard 
                                title="Files Discovered" 
                                value={status.total_items} 
                                subValue={`${(status.item_stats['queued_download'] || 0) + (status.item_stats['discovered'] || 0)} queued`}
                                icon={<Cloud className="text-indigo-500 w-6 h-6" />}
                            />
                            <StatCard 
                                title="In Progress" 
                                value={(status.item_stats['downloading'] || 0) + (status.item_stats['processing'] || 0) + (status.item_stats['uploading'] || 0) + (status.item_stats['local_finalizing'] || 0)} 
                                subValue={`${status.item_stats['uploading'] || 0} uploading`}
                                icon={<RefreshCw className="text-yellow-500 w-6 h-6" />}
                            />
                            <StatCard 
                                title="Completed" 
                                value={(status.item_stats['completed_telegram'] || 0) + (status.item_stats['completed_local'] || 0)} 
                                subValue={`${status.item_stats['failed'] || 0} failed`}
                                icon={<CheckCircle className="text-green-500 w-6 h-6" />}
                                valueColor="text-green-500"
                            />
                        </div>

                        {/* Detailed Pipeline Stages */}
                        <div className="bg-telegram-bg rounded-xl p-4 border border-telegram-divider">
                            <h3 className="text-sm font-semibold text-telegram-subtext mb-3 uppercase tracking-wide">Pipeline Stages</h3>
                            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-4 text-center">
                                <PipelineStage title="Queued" count={status.item_stats['queued_download'] || 0} />
                                <PipelineStage title="Downloading" count={status.item_stats['downloading'] || 0} />
                                <PipelineStage title="Processing" count={status.item_stats['processing'] || 0} />
                                <PipelineStage title="Uploading" count={status.item_stats['uploading'] || 0} />
                                <PipelineStage title="Done" count={(status.item_stats['completed_telegram'] || 0) + (status.item_stats['completed_local'] || 0)} highlight />
                                <PipelineStage title="Failed" count={status.item_stats['failed'] || 0} error />
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
};

const StatCard = ({ title, value, subValue, icon, valueColor = "text-telegram-text" }: any) => (
    <div className="bg-telegram-bg border border-telegram-divider rounded-xl p-4 flex flex-col justify-between">
        <div className="flex justify-between items-start mb-2">
            <span className="text-sm text-telegram-subtext font-medium">{title}</span>
            {icon}
        </div>
        <div>
            <div className={`text-2xl font-bold ${valueColor}`}>{value?.toLocaleString() || 0}</div>
            <div className="text-xs text-telegram-subtext/80 mt-1">{subValue}</div>
        </div>
    </div>
);

const PipelineStage = ({ title, count, highlight, error }: any) => (
    <div className={`p-3 rounded-lg ${highlight ? 'bg-green-500/10' : error ? 'bg-red-500/10' : 'bg-telegram-card'} border ${highlight ? 'border-green-500/20' : error ? 'border-red-500/20' : 'border-telegram-divider'}`}>
        <div className={`text-xl font-bold ${highlight ? 'text-green-500' : error ? 'text-red-500' : 'text-telegram-text'}`}>
            {count?.toLocaleString() || 0}
        </div>
        <div className={`text-[10px] font-medium uppercase tracking-wider mt-1 ${highlight ? 'text-green-500/70' : error ? 'text-red-500/70' : 'text-telegram-subtext'}`}>
            {title}
        </div>
    </div>
);
