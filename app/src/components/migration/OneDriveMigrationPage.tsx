import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMigrationContext } from '../../context/MigrationContext';
import { AutoMigrationCenter } from './AutoMigrationCenter';
import { AdvancedSettingsDrawer } from './AdvancedSettingsDrawer';
import { Cloud, Settings } from 'lucide-react';

const OneDriveMigrationContent: React.FC = () => {
    const { t } = useTranslation();
    const [isSettingsDrawerOpen, setIsSettingsDrawerOpen] = useState<boolean>(false);

    const {
        msAccount,
        currentJobDetail,
        itemProgress,
        loading,
        snapshotLoading,
        scanProgress,
        scanSnapshotItems,
        autoProfile,
        dailyQuota,
        migrationActivity,
        processingLogs,
        connectMicrosoft,
        switchMicrosoftAccount,
        updateAutoSettings,
        rescanAuto,
        resetAndRescanAuto,
        stopAutoScan,
        deleteMigrationItem,
        renameMigrationItem,
        syncSingleItem,
        syncScanSnapshotItem,
        clearProcessingLogs,
    } = useMigrationContext();

    const handleRefresh = () => {
        void rescanAuto();
    };

    const handleResetScan = () => {
        const confirmed = window.confirm(t(
            'migration.reset_scan_confirm',
            'Xóa toàn bộ kết quả đã quét và bắt đầu lại từ đầu?',
        ));
        if (confirmed) {
            void resetAndRescanAuto();
        }
    };

    const handleSwitchAccount = () => {
        void switchMicrosoftAccount().catch(() => undefined);
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
                            {t('migration.page_subtitle', 'Tự động quét và đồng bộ dữ liệu từ Microsoft OneDrive sang Telegram Drive')}
                        </p>
                    </div>
                </div>

                {msAccount && (
                    <button
                        onClick={() => setIsSettingsDrawerOpen(true)}
                        className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-semibold bg-slate-900 border border-slate-800 text-slate-300 hover:text-white hover:bg-slate-800 transition-colors"
                    >
                        <Settings className="w-4 h-4" />
                        {t('migration.advanced_settings', 'Tùy chọn Nâng cao')}
                    </button>
                )}
            </div>

            {/* Smart Auto Migration Center Only */}
            <AutoMigrationCenter
                msAccount={msAccount}
                autoProfile={autoProfile}
                dailyQuota={dailyQuota}
                currentJobDetail={currentJobDetail}
                itemProgress={itemProgress}
                migrationActivity={migrationActivity}
                processingLogs={processingLogs}
                loading={loading}
                snapshotLoading={snapshotLoading}
                scanProgress={scanProgress}
                scanSnapshotItems={scanSnapshotItems}
                onConnectMs={() => { void connectMicrosoft(); }}
                onSwitchMs={handleSwitchAccount}
                onOpenSettings={() => setIsSettingsDrawerOpen(true)}
                onRefresh={handleRefresh}
                onResetScan={handleResetScan}
                onStopScan={() => { void stopAutoScan(); }}
                onClearProcessingLogs={clearProcessingLogs}
                onDeleteItem={(jId, iId) => { void deleteMigrationItem(jId, iId); }}
                onRenameItem={(jId, iId, nName) => { void renameMigrationItem(jId, iId, nName); }}
                onSyncSingleItem={(jId, iId) => { void syncSingleItem(jId, iId); }}
                onSyncCheckpointItem={(sourceItemId) => { void syncScanSnapshotItem(sourceItemId); }}
            />


            {/* Advanced Settings Drawer */}
            {msAccount && (
                <AdvancedSettingsDrawer
                    isOpen={isSettingsDrawerOpen}
                    autoProfile={autoProfile}
                    loading={loading}
                    onClose={() => setIsSettingsDrawerOpen(false)}
                    onSaveSettings={(dId, dName, tDir) => { void updateAutoSettings(dId, dName, tDir); }}
                />
            )}
        </div>
    );
};

export const OneDriveMigrationPage: React.FC = () => <OneDriveMigrationContent />;
