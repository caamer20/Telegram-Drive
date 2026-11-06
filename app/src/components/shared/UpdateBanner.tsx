import { motion } from 'framer-motion';
import { useMotionPolicy } from '../../context/MotionContext';
import { Download, X, RefreshCw, Sparkles } from 'lucide-react';
import type { UpdateInstallPhase } from '../../services/updateReliability';
import { useTranslation } from 'react-i18next';

interface UpdateBannerProps {
    available: boolean;
    version: string | null;
    downloading: boolean;
    progress: number;
    phase: UpdateInstallPhase | null;
    managedByPackageManager: boolean;
    onUpdate: () => void;
    onDismiss: () => void;
}

export function UpdateBanner({
    available,
    version,
    downloading,
    progress,
    phase,
    managedByPackageManager,
    onUpdate,
    onDismiss
}: UpdateBannerProps) {
    const { t } = useTranslation();
    const { reducedMotion, transition } = useMotionPolicy();
    return (
        <>
            {available && (
                <motion.div
                    data-ui-motion
                    initial={reducedMotion ? false : { opacity: 0, y: -50 }}
                    transition={transition(160)}
                    animate={{ opacity: 1, y: 0 }}
                    className="fixed top-0 left-0 right-0 z-50 p-3 bg-gradient-to-r from-telegram-primary/90 via-blue-500/90 to-purple-500/90 backdrop-blur-sm shadow-lg"
                >
                    <div className="flex items-center justify-center gap-4 max-w-screen-lg mx-auto">
                        <Sparkles className="w-5 h-5 text-yellow-300 animate-pulse" />

                        <span className="text-white font-medium">
                            {downloading ? (
                                <>{phase === 'verifying' ? t('runtime.update_verifying') : phase === 'installing' ? t('runtime.update_installing') : t('runtime.update_downloading', {percent: progress})}</>
                            ) : (
                                <>{t('runtime.update_available', {version})}</>
                            )}
                        </span>

                        {downloading ? (
                            <div className="flex items-center gap-2">
                                <RefreshCw className="w-4 h-4 text-white animate-spin" />
                                <div className="w-32 h-2 bg-white/30 rounded-full overflow-hidden">
                                    <motion.div
                                        className="h-full bg-white rounded-full"
                                        data-ui-motion
                                        initial={reducedMotion ? false : { width: 0 }}
                                        transition={transition(120)}
                                        animate={{ width: `${progress}%` }}
                                    />
                                </div>
                            </div>
                        ) : (
                            <button
                                onClick={onUpdate}
                                className="flex items-center gap-2 px-4 py-1.5 bg-white text-telegram-primary font-semibold rounded-full hover:bg-white/90 transition-colors shadow-md"
                            >
                                <Download className="w-4 h-4" />
                                {managedByPackageManager ? t('files.open') : t('settings.update_restart')}
                            </button>
                        )}

                        {!downloading && (
                            <button
                                onClick={onDismiss}
                                className="p-1 text-white/70 hover:text-white transition-colors"
                                title={t('common.close')}
                            >
                                <X className="w-4 h-4" />
                            </button>
                        )}
                    </div>
                </motion.div>
            )}
        </>
    );
}
