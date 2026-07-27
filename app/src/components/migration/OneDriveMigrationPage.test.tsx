import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MigrationJobDetail } from '../../types';
import { OneDriveMigrationPage } from './OneDriveMigrationPage';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn().mockResolvedValue(() => undefined),
}));
vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('./SetupSection', () => ({ SetupSection: () => <div>setup</div> }));
vi.mock('./ActivityStream', () => ({ ActivityStream: () => <div>activity</div> }));
vi.mock('./FileTable', () => ({ FileTable: () => <div>files</div> }));
vi.mock('./ProgressPanel', () => ({
    ProgressPanel: ({ detail, onStart }: { detail: MigrationJobDetail; onStart: () => void }) => (
        <button onClick={onStart}>{`job-${detail.job.id}-${detail.job.state}`}</button>
    ),
}));

afterEach(() => {
    cleanup();
    invokeMock.mockReset();
});

const detail = (state: string): MigrationJobDetail => ({
    job: {
        id: 7,
        source_folder_id: 'root',
        source_folder_path: '/',
        telegram_destination_id: null,
        telegram_destination_name: 'Saved Messages',
        local_backup_dir: '/tmp/backup',
        workspace_dir: '/tmp/workspace',
        state,
        started_at: 1,
        discovered_folders: 1,
        completed_folders: 0,
        discovered_items: 1,
        completed_items: 0,
        failed_items: 0,
        waiting_items: 0,
        created_at: 1,
        updated_at: 2,
    },
    stats: {
        total_folders: 1,
        total_files: 1,
        total_bytes: 100,
        completed_telegram: 0,
        completed_local: 0,
        completed_bytes: 0,
        failed_files: 0,
        waiting_files: 0,
        pending_files: 1,
    },
    folders: [],
    files: [],
});

describe('OneDriveMigrationPage session resume', () => {
    it('restores the latest interrupted job and resumes that job', async () => {
        invokeMock.mockImplementation(async (command: string) => {
            if (command === 'cmd_migration_ms_status') {
                return { account_name: 'Test', account_email: 'test@example.com' };
            }
            if (command === 'cmd_migration_get_resumable_job') return 7;
            if (command === 'cmd_migration_get_status') {
                const resumed = invokeMock.mock.calls.some(([name]) => name === 'cmd_migration_resume');
                return detail(resumed ? 'running' : 'stopped');
            }
            if (command === 'cmd_migration_resume') return undefined;
            throw new Error(`Unexpected command: ${command}`);
        });

        render(<OneDriveMigrationPage />);

        const resumeButton = await screen.findByRole('button', { name: 'job-7-stopped' });
        fireEvent.click(resumeButton);

        await waitFor(() => {
            expect(invokeMock).toHaveBeenCalledWith('cmd_migration_resume', { jobId: 7 });
            expect(screen.getByRole('button', { name: 'job-7-running' })).toBeInTheDocument();
        });
    });
});
