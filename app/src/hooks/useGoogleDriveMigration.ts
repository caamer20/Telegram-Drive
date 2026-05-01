import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { toast } from "sonner";

export interface GoogleDriveImportItem {
	id: string;
	name: string;
	size?: number;
}

export interface MigrationItem {
	id: string;
	filename: string;
	status:
		| "pending"
		| "starting"
		| "downloading"
		| "uploading"
		| "done"
		| "error";
	progress: number;
	error?: string;
}

interface MigrationPayload {
	id: string;
	filename: string;
	status: string;
	percent: number;
	error: string | null;
}

export function useGoogleDriveMigration(activeFolderId: number | null) {
	const queryClient = useQueryClient();
	const [migrationQueue, setMigrationQueue] = useState<MigrationItem[]>([]);

	useEffect(() => {
		let unlisten: UnlistenFn | undefined;
		listen<MigrationPayload>("migration-progress", (event) => {
			const payload = event.payload;
			setMigrationQueue((q) => {
				const existing = q.find((i) => i.id === payload.id);
				if (existing) {
					return q.map((i) =>
						i.id === payload.id
							? {
									...i,
									status: payload.status as MigrationItem["status"],
									progress: payload.percent,
									error: payload.error || undefined,
								}
							: i,
					);
				}

				return [
					...q,
					{
						id: payload.id,
						filename: payload.filename,
						status: payload.status as MigrationItem["status"],
						progress: payload.percent,
						error: payload.error || undefined,
					},
				];
			});

			if (payload.status === "done") {
				queryClient.invalidateQueries({ queryKey: ["files", activeFolderId] });
			}
		}).then((fn) => {
			unlisten = fn;
		});
		return () => {
			unlisten?.();
		};
	}, [activeFolderId, queryClient]);

	const importFiles = async (
		accessToken: string,
		items: GoogleDriveImportItem[],
	) => {
		try {
			await invoke("cmd_gd_import_files", {
				accessToken,
				items,
				folderId: activeFolderId,
			});
			toast.success(`Migration completed for ${items.length} files`);
		} catch (e) {
			toast.error(`Migration failed: ${e}`);
		}
	};

	const clearFinished = () => {
		setMigrationQueue((q) =>
			q.filter((i) => i.status !== "done" && i.status !== "error"),
		);
	};

	return {
		migrationQueue,
		importFiles,
		clearFinished,
	};
}
