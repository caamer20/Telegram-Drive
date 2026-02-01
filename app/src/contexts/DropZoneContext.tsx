import { createContext, useContext } from 'react';

/**
 * DropZoneContext - Minimal context for future extensibility
 * 
 * With dragDropEnabled: false, we don't need position-based detection.
 * This is kept minimal for potential future use.
 */

interface DropZoneContextType {
    // Minimal interface - DOM events handle everything
}

const DropZoneContext = createContext<DropZoneContextType | null>({});

export function DropZoneProvider({ children }: { children: React.ReactNode }) {
    return (
        <DropZoneContext.Provider value={{}}>
            {children}
        </DropZoneContext.Provider>
    );
}

export function useDropZone() {
    const context = useContext(DropZoneContext);
    if (!context) {
        throw new Error('useDropZone must be used within DropZoneProvider');
    }
    return context;
}
