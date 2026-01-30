import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { Folder, Eye } from 'lucide-react';
import { TelegramFile } from '../../types';
import { FileTypeIcon } from '../FileTypeIcon';

interface FileCardProps {
    file: TelegramFile;
    onDelete: () => void;
    onDownload: () => void;
    onPreview?: () => void;
    isSelected: boolean;
    onClick?: (e: React.MouseEvent) => void;
}

export function FileCard({ file, onDelete, onDownload, onPreview, isSelected, onClick }: FileCardProps) {
    const isFolder = file.type === 'folder';
    const [showMenu, setShowMenu] = useState(false);

    // Handle Right Click
    const handleContextMenu = (e: React.MouseEvent) => {
        if (isFolder) return;
        e.preventDefault();
        e.stopPropagation();

        // If not selected, select it? Or just show menu?
        // Usually right click selects item if not selected.
        if (!isSelected && onClick) onClick(e);

        setShowMenu(true);
    };

    // Close menu when clicking elsewhere
    useEffect(() => {
        const close = () => setShowMenu(false);
        if (showMenu) window.addEventListener('click', close);
        return () => window.removeEventListener('click', close);
    }, [showMenu]);

    return (
        <div
            className="relative"
            onContextMenu={handleContextMenu}
            onClick={onClick}
        >
            <motion.div
                layout
                draggable={!isFolder}
                onDragStart={(e: any) => {
                    e.dataTransfer.setData("application/x-telegram-file-id", file.id.toString());
                }}
                whileHover={{ y: -4 }}
                className={`group cursor-pointer aspect-[4/3] bg-telegram-surface rounded-xl p-4 flex flex-col justify-between border hover:shadow-[0_4px_20px_rgba(0,0,0,0.2)] transition-all relative overflow-hidden ${isSelected ? 'border-telegram-primary bg-telegram-primary/5 ring-1 ring-telegram-primary' : 'border-telegram-border hover:border-telegram-primary/50'}`}
            >
                {/* Selection Checkmark */}
                <div className={`absolute top-2 left-2 w-5 h-5 rounded-full border flex items-center justify-center transition-all ${isSelected ? 'bg-telegram-primary border-telegram-primary' : 'border-telegram-border bg-telegram-bg/50 opacity-0 group-hover:opacity-100'}`}>
                    {isSelected && <div className="w-1.5 h-1.5 bg-black rounded-full" />}
                </div>

                <div className={`absolute top-0 right-0 p-2 opacity-0 group-hover:opacity-100 transition-opacity duration-300`}>
                    <div className="w-1.5 h-1.5 rounded-full bg-telegram-primary shadow-[0_0_8px_rgba(255,174,0,0.8)]"></div>
                </div>

                <div>
                    {isFolder ? <Folder className="w-10 h-10 text-telegram-primary" /> : <FileTypeIcon filename={file.name} />}
                </div>
                <div>
                    <h3 className="text-sm font-medium text-telegram-text truncate w-full" title={file.name}>{file.name}</h3>
                    <p className="text-xs text-telegram-subtext mt-1">{file.sizeStr}</p>
                </div>
                {/* Quick actions on hover */}
                <div className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity flex gap-1">
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview() }} className="p-1 bg-black/50 rounded-full hover:bg-telegram-primary hover:text-white text-white/70">
                        <Eye className="w-3 h-3" />
                    </button>
                </div>
            </motion.div>

            {/* Custom Context Menu - Outside overflow-hidden */}
            {showMenu && (
                <div className="absolute top-2 right-2 bg-telegram-surface border border-telegram-border rounded-lg shadow-xl z-50 flex flex-col w-32 overflow-hidden animate-in fade-in zoom-in-95 duration-100">
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview(); }} className="px-3 py-2 text-left text-xs text-telegram-text hover:bg-telegram-hover flex items-center gap-2">
                        <Eye className="w-3 h-3" /> Preview
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); onDownload(); }} className="px-3 py-2 text-left text-xs text-telegram-text hover:bg-telegram-hover flex items-center gap-2">
                        Download
                    </button>
                    <div className="h-px bg-telegram-border my-0"></div>
                    <button onClick={(e) => { e.stopPropagation(); onDelete(); }} className="px-3 py-2 text-left text-xs text-red-400 hover:bg-telegram-hover flex items-center gap-2">
                        Delete
                    </button>
                </div>
            )}
        </div>
    )
}
