import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProcessingLogPanel } from './ProcessingLogPanel';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

afterEach(cleanup);

describe('ProcessingLogPanel', () => {
    it('renders detailed log entries and clears them on request', () => {
        const onClear = vi.fn();
        render(
            <ProcessingLogPanel
                entries={[{
                    id: 'scan-1',
                    timestamp: Date.now(),
                    category: 'scan',
                    level: 'info',
                    message_key: 'migration.log_scan_progress',
                    params: { pages: 2, files: 100, folders: 8, seconds: 1 },
                }]}
                onClear={onClear}
            />,
        );

        expect(screen.getByText('migration.processing_log_title')).toBeInTheDocument();
        expect(screen.getByText('migration.log_scan_progress')).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: /migration.clear_log/i }));
        expect(onClear).toHaveBeenCalledOnce();
    });
});
