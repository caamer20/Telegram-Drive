import { useState } from 'react';
import { Plus } from 'lucide-react';

interface SidebarItemProps {
    icon: any;
    label: string;
    active: boolean;
    onClick: () => void;
    onDrop: (e: any) => void;
    onDelete?: () => void;
}

export function SidebarItem({ icon: Icon, label, active = false, onClick, onDrop, onDelete }: SidebarItemProps) {
    const [isOver, setIsOver] = useState(false);

    return (
        <button
            onClick={onClick}
            onDragOver={(e) => {
                e.preventDefault();
                setIsOver(true);
            }}
            onDragLeave={() => setIsOver(false)}
            onDrop={(e) => {
                setIsOver(false);
                if (onDrop) onDrop(e);
            }}
            onContextMenu={(e) => {
                if (onDelete) {
                    e.preventDefault();
                    onDelete();
                }
            }}
            className={`group w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition-colors ${active ? 'bg-telegram-primary/10 text-telegram-primary' : isOver ? 'bg-telegram-primary/20 text-telegram-text' : 'text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text'}`}
        >
            <Icon className="w-4 h-4" />
            <span className="flex-1 text-left truncate">{label}</span>
            {onDelete && (
                <div onClick={(e) => { e.stopPropagation(); onDelete(); }} className="opacity-0 group-hover:opacity-100 p-1 hover:text-red-400">
                    <Plus className="w-3 h-3 rotate-45" />
                </div>
            )}
        </button>
    )
}
