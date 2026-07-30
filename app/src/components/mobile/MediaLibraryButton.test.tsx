import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { openMediaLibrary, toastError } = vi.hoisted(() => ({
  openMediaLibrary: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../media-library', () => ({ openMediaLibrary }));
vi.mock('sonner', () => ({ toast: { error: toastError } }));

import { MediaLibraryButton } from './MediaLibraryButton';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('MediaLibraryButton', () => {
  beforeEach(() => {
    openMediaLibrary.mockReset();
    toastError.mockReset();
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders only on Android', () => {
    const { rerender } = render(<MediaLibraryButton isAndroid={false} />);
    expect(screen.queryByRole('button', { name: 'Open media gallery' })).not.toBeInTheDocument();

    rerender(<MediaLibraryButton isAndroid />);
    expect(screen.getByRole('button', { name: 'Open media gallery' })).toBeInTheDocument();
  });

  it('opens the native media library exactly once per click', async () => {
    openMediaLibrary.mockResolvedValueOnce({ exitReason: 'closed' });
    render(<MediaLibraryButton isAndroid />);

    fireEvent.click(screen.getByRole('button', { name: 'Open media gallery' }));

    await waitFor(() => expect(openMediaLibrary).toHaveBeenCalledOnce());
  });

  it('blocks repeated opens while the native screen is active and re-enables after resolve', async () => {
    const pending = deferred<{ exitReason: string }>();
    openMediaLibrary.mockReturnValueOnce(pending.promise);
    render(<MediaLibraryButton isAndroid />);
    const button = screen.getByRole('button', { name: 'Open media gallery' });

    fireEvent.click(button);
    fireEvent.click(button);

    expect(openMediaLibrary).toHaveBeenCalledOnce();
    expect(button).toBeDisabled();

    pending.resolve({ exitReason: 'back' });
    await waitFor(() => expect(button).toBeEnabled());
  });

  it('shows invoke failures and re-enables the button after reject', async () => {
    openMediaLibrary.mockRejectedValueOnce(new Error('invoke failed'));
    render(<MediaLibraryButton isAndroid />);
    const button = screen.getByRole('button', { name: 'Open media gallery' });

    fireEvent.click(button);

    await waitFor(() => expect(toastError).toHaveBeenCalledWith('Unable to open media gallery'));
    expect(button).toBeEnabled();
  });

  it('shows errors returned by the native screen', async () => {
    openMediaLibrary.mockResolvedValueOnce({ exitReason: 'error', error: 'Gallery failed' });
    render(<MediaLibraryButton isAndroid />);

    fireEvent.click(screen.getByRole('button', { name: 'Open media gallery' }));

    await waitFor(() => expect(toastError).toHaveBeenCalledWith('Gallery failed'));
  });

  it.each(['back', 'closed'])('does not show an error when the gallery exits with %s', async (exitReason) => {
    openMediaLibrary.mockResolvedValueOnce({ exitReason });
    render(<MediaLibraryButton isAndroid />);

    fireEvent.click(screen.getByRole('button', { name: 'Open media gallery' }));

    await waitFor(() => expect(openMediaLibrary).toHaveBeenCalledOnce());
    expect(toastError).not.toHaveBeenCalled();
  });
});
