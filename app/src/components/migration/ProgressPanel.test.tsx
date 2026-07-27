import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MigrationJobDetail } from '../../types';
import { ProgressPanel } from './ProgressPanel';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

afterEach(cleanup);

const detailForState = (state: string): MigrationJobDetail => ({
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
        discovered_items: 2,
        completed_items: 1,
        failed_items: 0,
        waiting_items: 0,
        created_at: 1,
        updated_at: 2,
    },
    stats: {
        total_folders: 1,
        total_files: 2,
        total_bytes: 200,
        completed_telegram: 1,
        completed_local: 0,
        completed_bytes: 100,
        failed_files: 0,
        waiting_files: 0,
        pending_files: 1,
    },
    folders: [],
    files: [],
});

describe('ProgressPanel resume control', () => {
    it('offers Resume for an interrupted job and invokes the resume action', () => {
        const onResume = vi.fn();
        render(
            <ProgressPanel
                detail={detailForState('stopped')}
                activeProgresses={{}}
                cooldown={null}
                onStart={onResume}
                onStop={vi.fn()}
                onRetryAllFailed={vi.fn()}
            />,
        );

        const resumeButton = screen.getByRole('button', { name: 'migration.btn_resume' });
        fireEvent.click(resumeButton);
        expect(onResume).toHaveBeenCalledOnce();
    });

    it('does not offer Resume while the job is already running', () => {
        render(
            <ProgressPanel
                detail={detailForState('running')}
                activeProgresses={{}}
                cooldown={null}
                onStart={vi.fn()}
                onStop={vi.fn()}
                onRetryAllFailed={vi.fn()}
            />,
        );

        expect(screen.queryByRole('button', { name: 'migration.btn_resume' })).not.toBeInTheDocument();
        expect(screen.getByRole('button', { name: 'migration.btn_stop' })).toBeInTheDocument();
    });
});
