import type {
    ItemProgressPayload,
    MigrationActivity,
    MigrationJobDetail,
    MigrationItem,
} from '../../types';

export interface TransferLists {
    downloading: MigrationItem[];
    uploading: MigrationItem[];
}

export function selectTransferLists(
    detail: MigrationJobDetail | null,
    progress: ItemProgressPayload | null,
): TransferLists {
    const empty: TransferLists = { downloading: [], uploading: [] };
    if (!detail) return empty;

    if (progress && detail.job.id === progress.job_id) {
        const item = detail.files.find(
            candidate => candidate.item_type === 'file' && candidate.id === progress.item_id,
        );
        if (item) {
            return progress.phase === 'downloading'
                ? { downloading: [item], uploading: [] }
                : { downloading: [], uploading: [item] };
        }
    }

    return {
        downloading: detail.files.filter(
            item => item.item_type === 'file' && item.state === 'downloading',
        ),
        uploading: detail.files.filter(
            item => item.item_type === 'file' && item.state === 'uploading',
        ),
    };
}

export function acceptProgressEvent(
    current: ItemProgressPayload | null,
    incoming: ItemProgressPayload,
): ItemProgressPayload {
    if (
        current?.job_id === incoming.job_id &&
        current.item_id === incoming.item_id
    ) {
        if (incoming.attempt < current.attempt) return current;
        if (incoming.attempt === current.attempt) {
            const phaseRank = { downloading: 0, uploading: 1 };
            if (phaseRank[incoming.phase] < phaseRank[current.phase]) return current;
            if (
                incoming.phase === current.phase &&
                incoming.revision <= current.revision
            ) {
                return current;
            }
        }
    }
    return incoming;
}

export function mergeActivity(
    current: MigrationActivity[],
    incoming: MigrationActivity,
): MigrationActivity[] {
    if (current.some(entry => entry.id === incoming.id)) return current;
    return [incoming, ...current].slice(0, 100);
}
