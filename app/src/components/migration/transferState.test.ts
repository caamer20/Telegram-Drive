import { describe, expect, it } from 'vitest';
import type {
    ItemProgressPayload,
    MigrationActivity,
    MigrationJobDetail,
    MigrationItem,
} from '../../types';
import { acceptProgressEvent, mergeActivity, selectTransferLists } from './transferState';

const item: MigrationItem = {
    id: 7,
    job_id: 3,
    item_type: 'file',
    name: 'report.pdf',
    source_path: 'report.pdf',
    size_bytes: 42,
    state: 'pending',
    attempt_count: 0,
    created_at: 1,
    queue_position: 0,
};

const detail = {
    job: { id: 3 },
    files: [item],
} as MigrationJobDetail;

function progress(
    phase: ItemProgressPayload['phase'],
    timestamp: number,
): ItemProgressPayload {
    return {
        job_id: 3,
        item_id: 7,
        item_name: item.name,
        phase,
        percent: 10,
        bytes_done: 4,
        bytes_total: 42,
        speed_bytes_per_sec: 0,
        event_id: `${phase}-${timestamp}`,
        attempt: 0,
        revision: timestamp,
        timestamp,
    };
}

describe('Auto Migration transfer state', () => {
    it('never places one item in both transfer lists', () => {
        expect(selectTransferLists(detail, progress('downloading', 1))).toEqual({
            downloading: [item],
            uploading: [],
        });
        expect(selectTransferLists(detail, progress('uploading', 2))).toEqual({
            downloading: [],
            uploading: [item],
        });
    });

    it('rejects progress for a file not in the authoritative job', () => {
        expect(
            selectTransferLists(detail, { ...progress('downloading', 1), item_id: 99 }),
        ).toEqual({ downloading: [], uploading: [] });
    });

    it('hydrates an active phase from persisted item state before the next event', () => {
        const hydrated = {
            ...detail,
            files: [{ ...item, state: 'uploading' }],
        } as MigrationJobDetail;
        expect(selectTransferLists(hydrated, null)).toEqual({
            downloading: [],
            uploading: [{ ...item, state: 'uploading' }],
        });
    });

    it('rejects an out-of-order progress event', () => {
        const current = progress('uploading', 20);
        expect(acceptProgressEvent(current, progress('downloading', 10))).toBe(current);
    });

    it('deduplicates persisted activity IDs', () => {
        const entry = {
            id: 1,
            job_id: 3,
            phase: 'downloading',
            status: 'started',
            created_at: 1,
        } as MigrationActivity;
        expect(mergeActivity([entry], entry)).toEqual([entry]);
    });
});
