export type TransferSortOrder = 'oldest' | 'newest';

interface DatedTransfer {
  id: string;
  createdAt?: number;
}

function creationTime(item: DatedTransfer): number | undefined {
  return typeof item.createdAt === 'number' && Number.isFinite(item.createdAt) && item.createdAt > 0
    ? item.createdAt
    : undefined;
}

/** Display-only ordering. Never mutate the queue, which also controls scheduling. */
export function sortTransfers<T extends DatedTransfer>(items: readonly T[], order: TransferSortOrder): T[] {
  return [...items].sort((a, b) => {
    const aTime = creationTime(a);
    const bTime = creationTime(b);
    // Historical dates cannot be reconstructed for legacy entries; keep them last.
    if (aTime === undefined && bTime !== undefined) return 1;
    if (aTime !== undefined && bTime === undefined) return -1;
    if (aTime !== undefined && bTime !== undefined && aTime !== bTime) {
      return order === 'oldest' ? aTime - bTime : bTime - aTime;
    }
    // Priority and progress can change array positions. IDs provide an immutable tie-break.
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
}
