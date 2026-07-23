import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AutoMigrationCenter } from './AutoMigrationCenter';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

afterEach(cleanup);

const baseProps = {
    msAccount: {
        account_name: 'OneDrive User',
        account_email: 'user@example.com',
    },
    autoProfile: null,
    dailyQuota: null,
    currentJobDetail: null,
    itemProgress: null,
    migrationActivity: [],
    loading: false,
    snapshotLoading: false,
    scanProgress: null,
    scanSnapshotItems: [],
    processingLogs: [],
    onToggleAuto: vi.fn(),
    onConnectMs: vi.fn(),
    onSwitchMs: vi.fn(),
    onOpenSettings: vi.fn(),
    onRefresh: vi.fn(),
    onResetScan: vi.fn(),
    onStopScan: vi.fn(),
    onClearProcessingLogs: vi.fn(),
    onSyncCheckpointItem: vi.fn(),
};

describe('AutoMigrationCenter account snapshot UX', () => {
    it('shows snapshot loading instead of the empty state while OneDrive is being scanned', () => {
        render(
            <AutoMigrationCenter
                {...baseProps}
                snapshotLoading
                scanProgress={{
                    phase: 'enumerating',
                    pages_scanned: 3,
                    discovered_files: 420,
                    discovered_folders: 18,
                    elapsed_ms: 2500,
                }}
            />,
        );

        expect(screen.getByText('migration.loading_onedrive_files')).toBeInTheDocument();
        expect(screen.getByText('migration.scan_progress_summary')).toBeInTheDocument();
        expect(screen.queryByText('migration.no_snapshot_files')).not.toBeInTheDocument();
    });

    it('allows switching Microsoft account from the OneDrive account card', () => {
        const onSwitchMs = vi.fn();
        render(<AutoMigrationCenter {...baseProps} onSwitchMs={onSwitchMs} />);

        fireEvent.click(screen.getByRole('button', { name: 'migration.switch_account' }));
        expect(onSwitchMs).toHaveBeenCalledOnce();
    });

    it('shows a stop action while scanning and a resume action after stopping', () => {
        const onStopScan = vi.fn();
        const onResetScan = vi.fn();
        const { rerender } = render(
            <AutoMigrationCenter
                {...baseProps}
                snapshotLoading
                onStopScan={onStopScan}
                scanProgress={{
                    phase: 'enumerating',
                    pages_scanned: 4,
                    discovered_files: 600,
                    discovered_folders: 20,
                    elapsed_ms: 3000,
                }}
            />,
        );

        fireEvent.click(screen.getByRole('button', { name: 'migration.stop_scan' }));
        expect(onStopScan).toHaveBeenCalledOnce();

        const onSyncCheckpointItem = vi.fn();
        rerender(
            <AutoMigrationCenter
                {...baseProps}
                onSyncCheckpointItem={onSyncCheckpointItem}
                onResetScan={onResetScan}
                scanProgress={{
                    phase: 'stopped',
                    pages_scanned: 4,
                    discovered_files: 600,
                    discovered_folders: 20,
                    elapsed_ms: 3000,
                }}
                scanSnapshotItems={[{
                    id: 'source-1',
                    name: 'checkpoint-file.txt',
                    item_type: 'file',
                    size: 42,
                    path: 'Documents/checkpoint-file.txt',
                    child_count: null,
                    etag: null,
                    quickxor_hash: null,
                    sha1_hash: null,
                    last_modified: null,
                }]}
            />,
        );
        expect(screen.getByRole('button', { name: 'migration.resume_scan' })).toBeInTheDocument();
        expect(screen.getByText('migration.scan_stopped_summary')).toBeInTheDocument();
        expect(screen.getByText('checkpoint-file.txt')).toBeInTheDocument();
        expect(screen.getByText('migration.scanned_checkpoint')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'migration.migrate_now' }));
        expect(onSyncCheckpointItem).toHaveBeenCalledWith('source-1');
        fireEvent.click(screen.getByRole('button', { name: 'migration.reset_scan' }));
        expect(onResetScan).toHaveBeenCalledOnce();
    });

    it('keeps pipeline controls disabled while backend startup is in progress', () => {
        render(
            <AutoMigrationCenter
                {...baseProps}
                snapshotLoading
                scanProgress={{
                    phase: 'starting',
                    pages_scanned: 0,
                    discovered_files: 0,
                    discovered_folders: 0,
                    elapsed_ms: 0,
                }}
            />,
        );

        const pipelineButtons = screen.getAllByRole('button', { name: 'migration.start_pipeline' });
        expect(pipelineButtons).toHaveLength(1);
        expect(pipelineButtons[0]).toBeDisabled();
        expect(screen.getByText('migration.pipeline_running')).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'migration.stop_scan' })).not.toBeInTheDocument();
    });
});
