import type {
    ItemProgressPayload,
    MigrationActivity,
    MigrationJobDetail,
    MigrationItem,
} from '../../types';

export interface TransferLists {
    downloading: MigrationItem[];
    processing: MigrationItem[];
    uploading: MigrationItem[];
}

export function selectTransferLists(
    detail: MigrationJobDetail | null,
    progress: ItemProgressPayload | null,
): TransferLists {
    const empty: TransferLists = { downloading: [], processing: [], uploading: [] };
    if (!detail) return empty;

    if (progress && detail.job.id === progress.job_id) {
        const item = detail.files.find(
            candidate => candidate.id === progress.item_id,
        );
        if (item) {
            if (progress.phase === 'downloading') {
                return { downloading: [item], processing: [], uploading: [] };
            }
            if (progress.phase === 'analyzing' || progress.phase === 'processing') {
                return { downloading: [], processing: [item], uploading: [] };
            }
            return { downloading: [], processing: [], uploading: [item] };
        }
    }

    return {
        downloading: detail.files.filter(
            item => item.pipeline_stage === 'downloading',
        ),
        processing: [],
        uploading: detail.files.filter(
            item => item.pipeline_stage === 'uploading',
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
            const phaseRank = { downloading: 0, analyzing: 1, processing: 2, uploading: 3 };
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
