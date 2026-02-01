/**
 * Simple file drop hook - Tauri native drag-drop is DISABLED
 * 
 * With dragDropEnabled: false, we have full control over DOM drag events.
 * Internal file moves work via SidebarItem's onDrop handler.
 * External file uploads use the Upload button (file picker dialog).
 */
export function useFileDrop() {
    return { isDragging: false };
}
