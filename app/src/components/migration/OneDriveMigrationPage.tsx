import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMigration } from '../../hooks/useMigration';
import { AutoMigrationCenter } from './AutoMigrationCenter';
import { AdvancedSettingsDrawer } from './AdvancedSettingsDrawer';
import { SetupSection } from './SetupSection';
import { ProgressPanel } from './ProgressPanel';
import { FileTable } from './FileTable';
import { Cloud, Plus, FolderKanban, SlidersHorizontal } from 'lucide-react';

export const OneDriveMigrationPage: React.FC = () => {
    const { t } = useTranslation();
    const [isSettingsDrawerOpen, setIsSettingsDrawerOpen] = useState<boolean>(false);
    const [showManualMode, setShowManualMode] = useState<boolean>(false);

    const {
        msAccount,
        jobs,
        currentJobDetail,
        itemProgress,
        cooldown,
        loading,
        autoProfile,
        dailyQuota,
        connectMicrosoft,
        disconnectMicrosoft,
        listOneDriveFolders,
        loadJob,
        createJob,
        setOneDriveFolder,
        setTelegramDestination,
        setLocalDir,
        scan,
        startMigration,
        pauseMigration,
        resumeMigration,
        cancelMigration,
        retryItem,
        retryAllFailed,
        toggleAuto,
        updateAutoSettings,
        getDailyQuota,
        getAutoStatus,
    } = useMigration();

    const handleRefresh = () => {
        getAutoStatus();
        getDailyQuota();
    };

    return (
        <div className="flex-1 h-full overflow-y-auto custom-scrollbar bg-slate-950 p-6 space-y-6 text-slate-100">
            {/* Header */}
            <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-slate-800">
                <div className="flex items-center gap-3">
                    <div className="p-3 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-xl shadow-lg shadow-blue-500/10">
                        <Cloud className="w-6 h-6 text-white" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold tracking-tight text-white">
                            {t('migration.page_title', 'OneDrive Migration')}
                        </h1>
                        <p className="text-xs text-slate-400">
                            {t('migration.page_subtitle', 'Tự động quét và đồng bộ dữ liệu từ Microsoft OneDrive sang Telegram')}
                        </p>
                    </div>
                </div>

                <div className="flex items-center gap-3">
                    <button
                        onClick={() => setShowManualMode(!showManualMode)}
                        className={`inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-semibold border transition-colors ${
                            showManualMode
                                ? 'bg-blue-600/10 border-blue-500/30 text-blue-400'
                                : 'bg-slate-900 border-slate-800 text-slate-400 hover:text-white'
                        }`}
                    >
                        <SlidersHorizontal className="w-4 h-4" />
                        {showManualMode ? 'Chế độ Tự Động (Auto)' : 'Chế độ Thủ Công (Manual)'}
                    </button>

                    {showManualMode && (
                        <button
                            onClick={createJob}
                            disabled={loading}
                            className="inline-flex items-center gap-2 px-4 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 border border-slate-700 rounded-lg text-xs font-semibold shadow transition-colors"
                        >
                            <Plus className="w-4 h-4" />
                            {t('migration.btn_new_job', 'New Job')}
                        </button>
                    )}
                </div>
            </div>

            {/* Smart Auto Migration Center (Default View) */}
            {!showManualMode ? (
                <AutoMigrationCenter
                    msAccount={msAccount}
                    autoProfile={autoProfile}
                    dailyQuota={dailyQuota}
                    currentJobDetail={currentJobDetail}
                    itemProgress={itemProgress}
                    loading={loading}
                    onToggleAuto={toggleAuto}
                    onConnectMs={() => connectMicrosoft()}
                    onOpenSettings={() => setIsSettingsDrawerOpen(true)}
                    onRefresh={handleRefresh}
                />
            ) : (
                <>
                    {/* Manual Mode Setup Section */}
                    <SetupSection
                        msAccount={msAccount}
                        currentDetail={currentJobDetail}
                        loading={loading}
                        onConnectMs={connectMicrosoft}
                        onDisconnectMs={disconnectMicrosoft}
                        onListOneDriveFolders={listOneDriveFolders}
                        onCreateJob={createJob}
                        onSetOneDriveFolder={(jobId, fId, fPath) => setOneDriveFolder(jobId, fId, fPath)}
                        onSetTelegramDest={(jobId, dId, dName) => setTelegramDestination(jobId, dId, dName)}
                        onSetLocalDir={(jobId, dir) => setLocalDir(jobId, dir)}
                        onScan={(jobId) => scan(jobId)}
                    />

                    {/* Progress Panel */}
                    {currentJobDetail && (
                        <ProgressPanel
                            detail={currentJobDetail}
                            itemProgress={itemProgress}
                            cooldown={cooldown}
                            loading={loading}
                            onStart={(jobId) => startMigration(jobId)}
                            onPause={(jobId) => pauseMigration(jobId)}
                            onResume={(jobId) => resumeMigration(jobId)}
                            onCancel={(jobId) => cancelMigration(jobId)}
                        />
                    )}

                    {/* File Table */}
                    {currentJobDetail && (
                        <FileTable
                            detail={currentJobDetail}
                            itemProgress={itemProgress}
                            loading={loading}
                            onRetryItem={(jobId, itemId) => retryItem(jobId, itemId)}
                            onRetryAllFailed={(jobId) => retryAllFailed(jobId)}
                        />
                    )}
                </>
            )}

            {/* Advanced Settings Drawer */}
            <AdvancedSettingsDrawer
                isOpen={isSettingsDrawerOpen}
                autoProfile={autoProfile}
                loading={loading}
                onClose={() => setIsSettingsDrawerOpen(false)}
                onSaveSettings={updateAutoSettings}
            />
        </div>
    );
};
