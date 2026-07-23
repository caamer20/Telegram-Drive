import React from 'react';
import { Cloud } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface ConnectionGateProps {
    loading: boolean;
    onConnect: () => void;
}

export const ConnectionGate: React.FC<ConnectionGateProps> = ({ loading, onConnect }) => {
    const { t } = useTranslation();
    return (
        <div className="min-h-[420px] flex items-center justify-center">
            <div className="max-w-md w-full bg-slate-900 border border-slate-800 rounded-2xl p-8 text-center shadow-xl">
                <div className="w-16 h-16 mx-auto rounded-2xl bg-blue-500/10 border border-blue-500/20 flex items-center justify-center">
                    <Cloud className="w-8 h-8 text-blue-400" />
                </div>
                <h2 className="mt-5 text-lg font-bold text-white">
                    {t('migration.connect_required')}
                </h2>
                <p className="mt-2 text-sm text-slate-400">
                    {t('migration.connect_required_description')}
                </p>
                <button
                    onClick={onConnect}
                    disabled={loading}
                    className="mt-6 px-5 py-2.5 rounded-xl bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white text-sm font-semibold transition-colors"
                >
                    {loading ? t('migration.connecting') : t('migration.connect_microsoft')}
                </button>
            </div>
        </div>
    );
};
