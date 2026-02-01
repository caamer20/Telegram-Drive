import { useState } from 'react';
import { Plus } from 'lucide-react';

interface SidebarItemProps {
    icon: React.ElementType;
    label: string;
    active: boolean;
    onClick: () => void;
    onDrop: (e: React.DragEvent) => void;
    onDelete?: () => void;
    folderId: number | null;
}

/**
 * SidebarItem - Pure DOM event-based drop handling
 * 
 * With Tauri's dragDropEnabled: false, DOM events work reliably.
 * This component handles internal file moves via standard React drag events.
 */
export function SidebarItem({ icon: Icon, label, active = false, onClick, onDrop, onDelete }: SidebarItemProps) {
    const [isOver, setIsOver] = useState(false);

    return (
        <button
            onClick={onClick}
            onDragEnter={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setIsOver(true);
            }}
            onDragOver={(e) => {
                e.preventDefault();
                e.stopPropagation();
                e.dataTransfer.dropEffect = 'move';
            }}
            onDragLeave={(e) => {
                e.preventDefault();
                e.stopPropagation();
                // Only clear if truly leaving (not entering a child element)
                const rect = e.currentTarget.getBoundingClientRect();
                const x = e.clientX;
                const y = e.clientY;
                if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) {
                    setIsOver(false);
                }
            }}
            onDrop={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setIsOver(false);
                console.log('=== SidebarItem DROP ===', label);
                if (onDrop) onDrop(e);
            }}
            onContextMenu={(e) => {
                if (onDelete) {
                    e.preventDefault();
                    onDelete();
                }
            }}
            className={`group w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition-all duration-150 ${active
                    ? 'bg-telegram-primary/10 text-telegram-primary'
                    : isOver
                        ? 'bg-telegram-primary/30 text-telegram-text ring-2 ring-telegram-primary scale-[1.02] shadow-lg'
                        : 'text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text'
                }`}
        >
            <Icon className={`w-4 h-4 ${isOver ? 'text-telegram-primary' : ''}`} />
            <span className="flex-1 text-left truncate">{label}</span>
            {onDelete && (
                <div onClick={(e) => { e.stopPropagation(); onDelete(); }} className="opacity-0 group-hover:opacity-100 p-1 hover:text-red-400">
                    <Plus className="w-3 h-3 rotate-45" />
                </div>
            )}
        </button>
    )
}
