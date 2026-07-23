import React, { createContext, useContext } from 'react';
import { useMigration } from '../hooks/useMigration';

type MigrationContextValue = ReturnType<typeof useMigration>;

const MigrationContext = createContext<MigrationContextValue | null>(null);

export const MigrationProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
    const value = useMigration();
    return <MigrationContext.Provider value={value}>{children}</MigrationContext.Provider>;
};

export function useMigrationContext(): MigrationContextValue {
    const value = useContext(MigrationContext);
    if (!value) {
        throw new Error('useMigrationContext must be used inside MigrationProvider');
    }
    return value;
}
