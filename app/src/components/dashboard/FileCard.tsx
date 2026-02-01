import { motion } from 'framer-motion';
import { useState } from 'react';
import { Folder, Eye, Trash2 } from 'lucide-react';
import { TelegramFile } from '../../types';
import { FileTypeIcon } from '../FileTypeIcon';

interface FileCardProps {
    file: TelegramFile;
    onDelete: () => void;
    onDownload: () => void;
    onPreview?: () => void;
    isSelected: boolean;
    onClick?: (e: React.MouseEvent) => void;
    onContextMenu?: (e: React.MouseEvent) => void;
    onDrop?: (e: React.DragEvent, folderId: number) => void;
    onDragStart?: (fileId: number) => void;
    onDragEnd?: () => void;
}

export function FileCard({ file, onDelete, onDownload, onPreview, isSelected, onClick, onContextMenu, onDrop, onDragStart, onDragEnd }: FileCardProps) {
    const isFolder = file.type === 'folder';
    const [isDragOver, setIsDragOver] = useState(false);

    return (
        <div
            className="relative"
            onContextMenu={onContextMenu}
            onClick={onClick}
            onDragOver={(e) => {
                if (isFolder) {
                    e.preventDefault();
                    e.stopPropagation();
                    if (!isDragOver) setIsDragOver(true);
                }
            }}
            onDragLeave={(e) => {
                if (isFolder) {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(false);
                }
            }}
            onDrop={(e) => {
                if (isFolder && onDrop) {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(false);
                    onDrop(e, file.id);
                }
            }}
        >
            <motion.div
                layout
                draggable={!isFolder}
                onDragStart={(e: any) => {
                    console.log('FileCard: OnDragStart', file.name, file.id);
                    if (onDragStart) onDragStart(file.id);
                    e.dataTransfer.setData("application/x-telegram-file-id", file.id.toString());
                    e.dataTransfer.effectAllowed = 'move';
                }}
                onDragEnd={() => {
                    if (onDragEnd) onDragEnd();
                }}
                whileHover={{ y: -4 }}
                className={`group cursor-pointer aspect-[4/3] bg-telegram-surface rounded-xl p-4 flex flex-col justify-between border hover:shadow-[0_4px_20px_rgba(0,0,0,0.2)] transition-all relative overflow-hidden 
                ${isSelected ? 'border-telegram-primary bg-telegram-primary/5 ring-1 ring-telegram-primary' : 'border-telegram-border hover:border-telegram-primary/50'}
                ${isDragOver ? 'ring-2 ring-telegram-primary bg-telegram-primary/20 scale-105' : ''}`}
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
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview() }} className="p-1 bg-black/50 rounded-full hover:bg-telegram-primary hover:text-white text-white/70" title="Preview">
                        <Eye className="w-3 h-3" />
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); onDownload() }} className="p-1 bg-black/50 rounded-full hover:bg-green-500 hover:text-white text-white/70" title="Download">
                        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-3 h-3"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line></svg>
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); onDelete() }} className="p-1 bg-black/50 rounded-full hover:bg-red-500 hover:text-white text-white/70" title="Delete">
                        <Trash2 className="w-3 h-3" />
                    </button>
                </div>
            </motion.div>
        </div>
    )
}
