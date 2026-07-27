import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { SetupSection } from './SetupSection';
import { ProgressPanel } from './ProgressPanel';
import { ActivityStream } from './ActivityStream';
import { FileTable } from './FileTable';
import { MigrationJobDetail, ItemProgressPayload, MigrationActivity, MsAccountInfo, OneDriveItem } from '../../types';
import { useTranslation } from 'react-i18next';
import { Play, RefreshCw, AlertTriangle } from 'lucide-react';

export const OneDriveMigrationPage: React.FC = () => {
    const { t } = useTranslation();
    
    // States
    const [msAccount, setMsAccount] = useState<MsAccountInfo | null>(null);
    const [currentDetail, setCurrentDetail] = useState<MigrationJobDetail | null>(null);
    const [progress, setProgress] = useState<ItemProgressPayload | null>(null);
    const [activities] = useState<MigrationActivity[]>([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    
    // Config state
    const [sourceId, setSourceId] = useState<string>('');
    const [sourcePath, setSourcePath] = useState<string>('');
    const [destId, setDestId] = useState<number | null>(null);
    const [destName, setDestName] = useState<string>('');
    const [localDir, setLocalDir] = useState<string>('');

    // Polling setup
    useEffect(() => {
        let isMounted = true;
        
        const fetchStatus = async () => {
            try {
                // MS Account status
                const msStatus = await invoke<MsAccountInfo>('cmd_migration_ms_status');
                if (isMounted) setMsAccount(msStatus);
                
                // Job status (assuming we fetch the active one if we don't know the ID yet)
                // For now, if currentDetail exists, we poll it.
                if (currentDetail?.job?.id) {
                    const detail = await invoke<MigrationJobDetail>('cmd_migration_get_status', { jobId: currentDetail.job.id });
                    if (isMounted) {
                        setCurrentDetail(detail);
                        // Also fetch items to update the file table? Wait, cmd_migration_get_status returns full MigrationJobDetail!
                    }
                } else {
                    // Try to fetch active job ID if available? No, wait for user to start or resume.
                }
            } catch (err: any) {
                console.error("Status fetch error", err);
            }
        };

        fetchStatus();
        const interval = setInterval(fetchStatus, 2000);
        return () => {
            isMounted = false;
            clearInterval(interval);
        };
    }, [currentDetail?.job?.id]);

    // Tauri Event listeners for progress
    useEffect(() => {
        let unlistenProgress: UnlistenFn | null = null;
        
        const setupListeners = async () => {
            unlistenProgress = await listen<ItemProgressPayload>('migration-item-progress', (event) => {
                setProgress(event.payload);
            });
        };
        
        setupListeners();
        return () => {
            if (unlistenProgress) unlistenProgress();
        };
    }, []);

    const handleConnectMs = async (clientId?: string, tenant?: string) => {
        setLoading(true);
        try {
            await invoke('cmd_migration_ms_connect', { clientId, tenantId: tenant });
            const msStatus = await invoke<MsAccountInfo>('cmd_migration_ms_status');
            setMsAccount(msStatus);
        } catch (e: any) {
            setError(e.toString());
        } finally {
            setLoading(false);
        }
    };

    const handleDisconnectMs = async () => {
        setLoading(true);
        try {
            await invoke('cmd_migration_ms_disconnect');
            setMsAccount(null);
        } catch (e: any) {
            setError(e.toString());
        } finally {
            setLoading(false);
        }
    };

    const handleListFolders = async (parentId?: string) => {
        return invoke<OneDriveItem[]>('cmd_migration_get_folder_children', { parentId: parentId || null });
    };

    const handleSetFolder = (folderId: string, path: string) => {
        setSourceId(folderId);
        setSourcePath(path);
    };

    const handleSetTelegram = (dId: number | null, dName: string) => {
        setDestId(dId);
        setDestName(dName);
    };

    const handleSetLocalDir = (dir: string) => {
        setLocalDir(dir);
    };

    const handleStart = async () => {
        if (!sourceId || !destName || !localDir) {
            setError("Please fill all required settings before starting.");
            return;
        }
        setLoading(true);
        setError(null);
        try {
            const jobId = await invoke<number>('cmd_migration_start', {
                sourceFolderId: sourceId,
                sourceFolderPath: sourcePath,
                telegramDestinationId: destId,
                telegramDestinationName: destName,
                localBackupDir: localDir
            });
            const detail = await invoke<MigrationJobDetail>('cmd_migration_get_status', { jobId });
            setCurrentDetail(detail);
        } catch (e: any) {
            setError(e.toString());
        } finally {
            setLoading(false);
        }
    };

    const handleStop = async () => {
        try {
            await invoke('cmd_migration_stop');
        } catch (e: any) {
            setError(e.toString());
        }
    };

    const handleRetry = async () => {
        if (!currentDetail?.job?.id) return;
        try {
            await invoke('cmd_migration_retry_failed', { jobId: currentDetail.job.id });
        } catch (e: any) {
            setError(e.toString());
        }
    };
    
    // We determine if we are in Setup phase or Execution phase
    const showSetup = !currentDetail || currentDetail.job.state === 'failed' || currentDetail.job.state === 'completed';

    return (
        <div className="h-full flex flex-col bg-slate-950 text-slate-200 overflow-hidden">
            {/* Header */}
            <header className="flex-none px-6 py-4 border-b border-slate-800/60 bg-slate-900/50 flex justify-between items-center z-10 backdrop-blur-sm">
                <div className="flex items-center gap-3">
                    <div className="p-2 bg-blue-500/10 rounded-lg border border-blue-500/20">
                        <RefreshCw className="w-5 h-5 text-blue-400" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-blue-400 to-indigo-300">
                            {t('migration.title', 'OneDrive Migration')}
                        </h1>
                        <p className="text-xs text-slate-500 font-medium">Seamless cloud to telegram sync</p>
                    </div>
                </div>
            </header>

            <main className="flex-1 overflow-y-auto custom-scrollbar p-6 space-y-6">
                {error && (
                    <div className="p-4 bg-red-500/10 border border-red-500/20 rounded-xl flex items-start gap-3">
                        <AlertTriangle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
                        <div className="text-sm text-red-200">{error}</div>
                        <button onClick={() => setError(null)} className="ml-auto text-red-400 hover:text-red-300 text-xs uppercase tracking-wider font-bold">Dismiss</button>
                    </div>
                )}

                {showSetup ? (
                    <div className="max-w-4xl mx-auto space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
                        <SetupSection
                            msAccount={msAccount}
                            loading={loading}
                            sourceFolderPath={sourcePath}
                            telegramDestName={destName}
                            localDir={localDir}
                            onConnectMs={handleConnectMs}
                            onDisconnectMs={handleDisconnectMs}
                            onListOneDriveFolders={handleListFolders}
                            onSetSourceFolder={handleSetFolder}
                            onSetTelegramDest={handleSetTelegram}
                            onSetLocalDir={handleSetLocalDir}
                        />
                        
                        {/* Start Button Area */}
                        {msAccount && sourceId && destName && localDir && (
                            <div className="flex justify-end pt-4 border-t border-slate-800">
                                <button
                                    onClick={handleStart}
                                    disabled={loading}
                                    className="flex items-center gap-2 px-8 py-3 bg-gradient-to-r from-blue-600 to-indigo-600 hover:from-blue-500 hover:to-indigo-500 text-white rounded-xl font-semibold shadow-lg shadow-blue-500/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed transform hover:-translate-y-0.5 active:translate-y-0"
                                >
                                    {loading ? <RefreshCw className="w-5 h-5 animate-spin" /> : <Play className="w-5 h-5" />}
                                    {t('migration.start', 'Start Migration')}
                                </button>
                            </div>
                        )}
                    </div>
                ) : (
                    <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 h-full animate-in fade-in duration-500">
                        {/* Left Column: Progress & Stats */}
                        <div className="lg:col-span-2 space-y-6 flex flex-col h-full">
                            {currentDetail && (
                                <ProgressPanel
                                    detail={currentDetail}
                                    progress={progress}
                                    cooldown={null}
                                    onStart={handleStart}
                                    onStop={handleStop}
                                    onRetryAllFailed={handleRetry}
                                />
                            )}
                            
                            {/* File Table / Queue */}
                            <div className="flex-1 min-h-[300px] bg-slate-900/60 rounded-xl border border-slate-800/60 overflow-hidden flex flex-col">
                                <div className="p-4 border-b border-slate-800/60 bg-slate-900/80">
                                    <h3 className="font-semibold text-slate-300">File Queue</h3>
                                </div>
                                <div className="flex-1 overflow-hidden">
                                    {currentDetail?.files && <FileTable files={currentDetail.files} onRetryItem={(itemId) => { console.log("Retry", itemId); }} />}
                                </div>
                            </div>
                        </div>

                        {/* Right Column: Logs */}
                        <div className="flex flex-col h-full space-y-6">
                            <div className="flex-1 bg-slate-900/60 rounded-xl border border-slate-800/60 overflow-hidden flex flex-col">
                                <div className="p-4 border-b border-slate-800/60 bg-slate-900/80">
                                    <h3 className="font-semibold text-slate-300">Activity Log</h3>
                                </div>
                                <div className="flex-1 overflow-y-auto custom-scrollbar p-2">
                                    <ActivityStream entries={activities} />
                                </div>
                            </div>
                        </div>
                    </div>
                )}
            </main>
        </div>
    );
};
