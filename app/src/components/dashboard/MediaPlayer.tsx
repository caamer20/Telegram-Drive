import { X } from 'lucide-react';
import { TelegramFile } from '../../types';

interface MediaPlayerProps {
    file: TelegramFile;
    onClose: () => void;
    activeFolderId: number | null;
}

export function MediaPlayer({ file, onClose, activeFolderId }: MediaPlayerProps) {
    const folderIdParam = activeFolderId !== null ? activeFolderId.toString() : 'home';
    const streamUrl = `http://localhost:14200/stream/${folderIdParam}/${file.id}`;

    const isVideo = ['mp4', 'webm', 'ogg', 'mov', 'mkv', 'avi'].some(ext => file.name.toLowerCase().endsWith(ext));
    const isAudio = ['mp3', 'wav', 'aac', 'flac', 'm4a', 'opus'].some(ext => file.name.toLowerCase().endsWith(ext));

    return (
        <div className="fixed inset-0 z-[200] bg-black/90 flex items-center justify-center p-4 backdrop-blur-md animate-in fade-in duration-200" onClick={onClose}>
            <div className="relative w-full max-w-6xl flex flex-col items-center" onClick={e => e.stopPropagation()}>
                <button
                    onClick={onClose}
                    className="absolute -top-12 right-0 p-2 text-white/50 hover:text-white bg-white/10 hover:bg-white/20 rounded-full transition-all"
                >
                    <X className="w-6 h-6" />
                </button>

                <div className="w-full aspect-video bg-black rounded-xl overflow-hidden shadow-2xl ring-1 ring-white/10 flex items-center justify-center">
                    {isVideo ? (
                        <video
                            src={streamUrl}
                            controls
                            autoPlay
                            className="w-full h-full object-contain"
                        />
                    ) : isAudio ? (
                        <div className="w-full h-full flex flex-col items-center justify-center bg-gradient-to-br from-telegram-primary/20 to-black">
                            <div className="w-32 h-32 rounded-full bg-telegram-surface flex items-center justify-center mb-8 shadow-xl animate-pulse-slow">
                                <svg xmlns="http://www.w3.org/2000/svg" className="w-12 h-12 text-telegram-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
                            </div>
                            <audio src={streamUrl} controls autoPlay className="w-full max-w-md" />
                        </div>
                    ) : (
                        <div className="text-white">Unsupported media type</div>
                    )}
                </div>

                <div className="mt-4 text-center">
                    <h3 className="text-lg font-medium text-white">{file.name}</h3>
                    <p className="text-sm text-white/50">Streaming from Telegram Drive</p>
                </div>
            </div>
        </div>
    );
}
