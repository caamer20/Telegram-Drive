import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { X, Folder, Send, Save, HardDrive } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { AutoMigrationProfile } from '../../types';

interface AdvancedSettingsDrawerProps {
    isOpen: boolean;
    autoProfile: AutoMigrationProfile | null;
    loading: boolean;
    onClose: () => void;
    onSaveSettings: (destId?: number, destName?: string, tempDir?: string) => void;
}

export const AdvancedSettingsDrawer: React.FC<AdvancedSettingsDrawerProps> = ({
    isOpen,
    autoProfile,
    loading,
    onClose,
    onSaveSettings,
}) => {
    const { t } = useTranslation();
    const [destId, setDestId] = useState<string>(autoProfile?.default_telegram_dest_id?.toString() || '');
    const [destName, setDestName] = useState<string>(autoProfile?.default_telegram_dest_name || '');
    const [tempDir, setTempDir] = useState<string>(autoProfile?.local_temp_dir || '');

    useEffect(() => {
        if (autoProfile) {
            setDestId(autoProfile.default_telegram_dest_id?.toString() || '');
            setDestName(autoProfile.default_telegram_dest_name || '');
            setTempDir(autoProfile.local_temp_dir || '');
        }
    }, [autoProfile]);

    if (!isOpen) return null;

    const handleBrowseFolder = async () => {
        try {
            const selected = await open({
                directory: true,
                multiple: false,
                title: 'Select Local Temp Folder for Migration',
            });
            if (selected && typeof selected === 'string') {
                setTempDir(selected);
            }
        } catch (e) {
            console.error('Failed to open folder picker:', e);
        }
    };

    const handleSave = () => {
        const parsedDestId = destId ? parseInt(destId, 10) : undefined;
        onSaveSettings(parsedDestId, destName || undefined, tempDir || undefined);
        onClose();
    };

    return (
        <div className="fixed inset-0 z-50 overflow-hidden bg-slate-950/70 backdrop-blur-sm flex justify-end">
            <div className="w-full max-w-md bg-slate-900 border-l border-slate-800 h-full p-6 flex flex-col justify-between shadow-2xl animate-in slide-in-from-right duration-200">
                <div className="space-y-6">
                    <div className="flex items-center justify-between pb-4 border-b border-slate-800">
                        <div className="flex items-center gap-2">
                            <HardDrive className="w-5 h-5 text-blue-400" />
                            <h3 className="text-lg font-bold text-white">
                                {t('migration.advanced_settings', 'Tùy chọn Nâng cao')}
                            </h3>
                        </div>
                        <button
                            onClick={onClose}
                            className="p-1 hover:bg-slate-800 text-slate-400 hover:text-white rounded-lg transition-colors"
                        >
                            <X className="w-5 h-5" />
                        </button>
                    </div>

                    <div className="space-y-4">
                        {/* Telegram Destination */}
                        <div className="space-y-2">
                            <label className="text-xs font-semibold text-slate-300 flex items-center gap-1.5">
                                <Send className="w-4 h-4 text-emerald-400" />
                                Kênh Telegram nhận file mặc định
                            </label>
                            <input
                                type="text"
                                value={destName}
                                onChange={(e) => setDestName(e.target.value)}
                                placeholder="Tên kênh (ví dụ: Saved Messages)"
                                className="w-full px-3 py-2 bg-slate-950 border border-slate-800 rounded-xl text-xs text-slate-200 focus:outline-none focus:border-blue-500"
                            />
                            <input
                                type="number"
                                value={destId}
                                onChange={(e) => setDestId(e.target.value)}
                                placeholder="ID Kênh Telegram (ví dụ: -100123456789)"
                                className="w-full px-3 py-2 bg-slate-950 border border-slate-800 rounded-xl text-xs text-slate-200 focus:outline-none focus:border-blue-500"
                            />
                        </div>

                        {/* Local Temp Directory */}
                        <div className="space-y-2 pt-2 border-t border-slate-800/60">
                            <label className="text-xs font-semibold text-slate-300 flex items-center gap-1.5">
                                <Folder className="w-4 h-4 text-sky-400" />
                                Thư mục tạm trên ổ đĩa local
                            </label>
                            <div className="flex gap-2">
                                <input
                                    type="text"
                                    value={tempDir}
                                    onChange={(e) => setTempDir(e.target.value)}
                                    placeholder="Tự động chọn thư mục mặc định hệ thống"
                                    className="w-full px-3 py-2 bg-slate-950 border border-slate-800 rounded-xl text-xs text-slate-200 focus:outline-none focus:border-blue-500"
                                />
                                <button
                                    onClick={handleBrowseFolder}
                                    className="px-3 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded-xl text-xs font-medium shrink-0 transition-colors"
                                >
                                    Duyệt
                                </button>
                            </div>
                            <p className="text-[11px] text-slate-500">
                                File tạm sẽ tự động bị xóa sạch ngay sau khi upload thành công.
                            </p>
                        </div>
                    </div>
                </div>

                <div className="pt-4 border-t border-slate-800 flex gap-3">
                    <button
                        onClick={onClose}
                        className="w-full py-2.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-xl text-xs font-semibold transition-colors"
                    >
                        {t('common.cancel', 'Hủy')}
                    </button>
                    <button
                        onClick={handleSave}
                        disabled={loading}
                        className="w-full py-2.5 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-600/20 transition-all flex items-center justify-center gap-2"
                    >
                        <Save className="w-4 h-4" />
                        {t('common.save', 'Lưu Cấu Hình')}
                    </button>
                </div>
            </div>
        </div>
    );
};
