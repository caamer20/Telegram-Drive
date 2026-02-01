import { Upload } from 'lucide-react';

interface EmptyStateProps {
    onUpload: () => void;
}

export function EmptyState({ onUpload }: EmptyStateProps) {
    return (
        <div className="flex flex-col items-center justify-center py-20 px-8 text-center">
            {/* Custom SVG Illustration */}
            <svg
                className="w-48 h-48 mb-8"
                viewBox="0 0 200 200"
                fill="none"
            >
                {/* Cloud shape - Light: Blue-ish tint, Dark: Subtle overlay */}
                <ellipse cx="100" cy="120" rx="70" ry="40" className="fill-blue-100 dark:fill-telegram-primary/5 opacity-50 dark:opacity-30" />

                {/* Folder base - Light: White with Blue Border, Dark: Dark Blue with Border */}
                <path
                    d="M40 80 L40 140 C40 145 44 150 50 150 L150 150 C156 150 160 145 160 140 L160 80 Z"
                    className="fill-white dark:fill-[#1e3a5f] stroke-blue-200 dark:stroke-telegram-primary/30"
                    strokeWidth="1"
                />

                {/* Folder tab */}
                <path
                    d="M40 80 L40 70 C40 65 44 60 50 60 L80 60 L90 70 L90 80 Z"
                    className="fill-white dark:fill-[#1e3a5f] stroke-blue-200 dark:stroke-telegram-primary/30"
                    strokeWidth="1"
                />

                {/* Plus icon in center */}
                <circle cx="100" cy="110" r="20" className="fill-blue-50 dark:fill-telegram-primary/10 stroke-blue-300 dark:stroke-telegram-primary/50" strokeWidth="2" strokeDasharray="4 2" />
                <path d="M100 100 L100 120 M90 110 L110 110" className="stroke-telegram-primary" strokeWidth="2" strokeLinecap="round" />

                {/* Floating documents */}
                <g className="animate-pulse">
                    <rect x="130" y="50" width="25" height="30" rx="3" className="fill-blue-500" />
                    <rect x="135" y="56" width="15" height="2" rx="1" className="fill-white/80" />
                    <rect x="135" y="62" width="12" height="2" rx="1" className="fill-white/80" />
                </g>

                <g opacity="0.6">
                    <rect x="45" y="40" width="20" height="25" rx="3" className="fill-gray-300 dark:fill-gray-500" />
                    <rect x="49" y="45" width="12" height="2" rx="1" className="fill-white/80" />
                    <rect x="49" y="50" width="8" height="2" rx="1" className="fill-white/80" />
                </g>
            </svg>

            <h3 className="text-xl font-semibold text-telegram-text mb-2">
                This folder is empty
            </h3>
            <p className="text-telegram-subtext text-sm mb-6 max-w-xs">
                Drag and drop files here, or click the button below to upload from your computer.
            </p>

            <button
                onClick={onUpload}
                className="inline-flex items-center gap-2 px-6 py-3 bg-telegram-primary text-black font-medium rounded-xl hover:bg-telegram-primary/90 transition-all hover:scale-105 shadow-lg shadow-telegram-primary/20"
            >
                <Upload className="w-5 h-5" />
                Upload Files
            </button>

            <p className="text-xs text-telegram-subtext/50 mt-6">
                Tip: Use <kbd className="px-1.5 py-0.5 bg-telegram-hover rounded text-telegram-subtext">Cmd + F</kbd> to search
            </p>
        </div>
    );
}
