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
        const incAttempt = incoming.attempt ?? 0;
        const curAttempt = current.attempt ?? 0;
        if (incAttempt < curAttempt) return current;
        if (incAttempt === curAttempt) {
            const phaseRank = { downloading: 0, analyzing: 1, processing: 2, uploading: 3 };
            if (phaseRank[incoming.phase] < phaseRank[current.phase]) return current;
            const incRevision = incoming.revision ?? 0;
            const curRevision = current.revision ?? 0;
            if (
                incoming.phase === current.phase &&
                incRevision <= curRevision
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
