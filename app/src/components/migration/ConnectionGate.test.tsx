import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ConnectionGate } from './ConnectionGate';

vi.mock('react-i18next', () => ({
    useTranslation: () => ({ t: (key: string) => key }),
}));

describe('ConnectionGate', () => {
    it('shows only the connection action before Microsoft is connected', () => {
        const onConnect = vi.fn();
        render(<ConnectionGate loading={false} onConnect={onConnect} />);

        expect(screen.getByText('migration.connect_required')).toBeInTheDocument();
        expect(screen.queryByText('migration.download_list')).not.toBeInTheDocument();
        expect(screen.queryByText('migration.upload_list')).not.toBeInTheDocument();
        expect(screen.queryByText('migration.recent_activity')).not.toBeInTheDocument();

        fireEvent.click(screen.getByRole('button'));
        expect(onConnect).toHaveBeenCalledOnce();
    });
});
