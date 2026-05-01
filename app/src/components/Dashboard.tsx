import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useFileDownload } from "../hooks/useFileDownload";
import { useFileOperations } from "../hooks/useFileOperations";
import { useFileUpload } from "../hooks/useFileUpload";
import { useGoogleDriveMigration } from "../hooks/useGoogleDriveMigration";
import { useKeyboardShortcuts } from "../hooks/useKeyboardShortcuts";
// Hooks
import { useTelegramConnection } from "../hooks/useTelegramConnection";
import type { BandwidthStats, TelegramFile } from "../types";
import { formatBytes, isMediaFile, isPdfFile } from "../utils";
import { DownloadQueue } from "./dashboard/DownloadQueue";
import { DragDropOverlay } from "./dashboard/DragDropOverlay";
import { ExternalDropBlocker } from "./dashboard/ExternalDropBlocker";
import { FileExplorer } from "./dashboard/FileExplorer";
import { GoogleDriveImportModal } from "./dashboard/GoogleDriveImportModal";
import { MediaPlayer } from "./dashboard/MediaPlayer";
import { MigrationQueue } from "./dashboard/MigrationQueue";
import { MoveToFolderModal } from "./dashboard/MoveToFolderModal";
import { PdfViewer } from "./dashboard/PdfViewer";
import { PreviewModal } from "./dashboard/PreviewModal";
// Components
import { Sidebar } from "./dashboard/Sidebar";
import { TopBar } from "./dashboard/TopBar";
import { UploadQueue } from "./dashboard/UploadQueue";

interface PendingGoogleDriveImport {
	token: string;
	items: any[];
	duplicates: any[];
}

export function Dashboard({ onLogout }: { onLogout: () => void }) {
	const queryClient = useQueryClient();

	const {
		store,
		folders,
		activeFolderId,
		setActiveFolderId,
		isSyncing,
		isConnected,
		handleLogout,
		handleSyncFolders,
		handleCreateFolder,
		handleFolderDelete,
	} = useTelegramConnection(onLogout);

	const [previewFile, setPreviewFile] = useState<TelegramFile | null>(null);
	const [viewMode, setViewMode] = useState<"grid" | "list">("grid");
	const [selectedIds, setSelectedIds] = useState<number[]>([]);
	const [showMoveModal, setShowMoveModal] = useState(false);
	const [showGoogleDriveImportModal, setShowGoogleDriveImportModal] =
		useState(false);
	const [pendingGoogleDriveImport, setPendingGoogleDriveImport] =
		useState<PendingGoogleDriveImport | null>(null);
	const [searchTerm, setSearchTerm] = useState("");
	const [searchResults, setSearchResults] = useState<TelegramFile[]>([]);
	const [isSearching, setIsSearching] = useState(false);
	const [internalDragFileId, _setInternalDragFileId] = useState<number | null>(
		null,
	);
	const internalDragRef = useRef<number | null>(null);

	const setInternalDragFileId = (id: number | null) => {
		internalDragRef.current = id;
		_setInternalDragFileId(id);
	};
	const [playingFile, setPlayingFile] = useState<TelegramFile | null>(null);
	const [pdfFile, setPdfFile] = useState<TelegramFile | null>(null);
	const [previewContextFiles, setPreviewContextFiles] = useState<
		TelegramFile[]
	>([]);
	const [previewContextIndex, setPreviewContextIndex] = useState(-1);

	useEffect(() => {
		if (store) {
			store.get<"grid" | "list">("viewMode").then((saved) => {
				if (saved) setViewMode(saved);
			});
		}
	}, [store]);

	useEffect(() => {
		if (store) {
			store.set("viewMode", viewMode).then(() => store.save());
		}
	}, [store, viewMode]);

	const {
		data: allFiles = [],
		isLoading,
		error,
	} = useQuery({
		queryKey: ["files", activeFolderId],
		queryFn: () =>
			invoke<any[]>("cmd_get_files", { folderId: activeFolderId }).then((res) =>
				res.map((f) => ({
					...f,
					sizeStr: formatBytes(f.size),
					type: f.icon_type || (f.name.endsWith("/") ? "folder" : "file"),
				})),
			),
		enabled: !!store,
	});

	const displayedFiles =
		searchTerm.length > 2
			? searchResults
			: allFiles.filter((f: TelegramFile) =>
					f.name.toLowerCase().includes(searchTerm.toLowerCase()),
				);

	const { data: bandwidth } = useQuery({
		queryKey: ["bandwidth"],
		queryFn: () => invoke<BandwidthStats>("cmd_get_bandwidth"),
		refetchInterval: 5000,
		enabled: !!store,
	});

	const {
		handleDelete,
		handleBulkDelete,
		handleBulkDownload,
		handleBulkMove,
		handleDownloadFolder,
		handleGlobalSearch,
	} = useFileOperations(
		activeFolderId,
		selectedIds,
		setSelectedIds,
		displayedFiles,
	);

	const {
		uploadQueue,
		setUploadQueue,
		handleManualUpload,
		cancelAll: cancelUploads,
		isDragging,
	} = useFileUpload(activeFolderId, store);
	const {
		downloadQueue,
		queueDownload,
		clearFinished: clearDownloads,
		cancelAll: cancelDownloads,
	} = useFileDownload(store);
	const {
		migrationQueue,
		importFiles,
		clearFinished: clearFinishedMigrations,
	} = useGoogleDriveMigration(activeFolderId);

	const handleSelectAll = useCallback(() => {
		setSelectedIds(displayedFiles.map((f) => f.id));
	}, [displayedFiles]);

	const handleKeyboardDelete = useCallback(() => {
		if (selectedIds.length > 0) {
			handleBulkDelete();
		}
	}, [selectedIds, handleBulkDelete]);

	const handleEscape = useCallback(() => {
		setSelectedIds([]);
		setSearchTerm("");
		setPreviewFile(null);
		setPlayingFile(null);
		setPdfFile(null);
	}, []);

	const handleFocusSearch = useCallback(() => {
		const searchInput = document.querySelector(
			'input[placeholder="Search files..."]',
		) as HTMLInputElement;
		if (searchInput) {
			searchInput.focus();
			searchInput.select();
		}
	}, []);

	const handleEnter = useCallback(() => {
		if (selectedIds.length === 1) {
			const selected = displayedFiles.find((f) => f.id === selectedIds[0]);
			if (selected) {
				if (selected.type === "folder") {
					setActiveFolderId(selected.id);
				} else {
					handlePreview(selected, displayedFiles);
				}
			}
		}
	}, [selectedIds, displayedFiles, setActiveFolderId]);

	useKeyboardShortcuts({
		onSelectAll: handleSelectAll,
		onDelete: handleKeyboardDelete,
		onEscape: handleEscape,
		onSearch: handleFocusSearch,
		onEnter: handleEnter,
		enabled: !previewFile && !playingFile && !pdfFile && !showMoveModal, // Disable when modals are open
	});

	useEffect(() => {
		setSelectedIds([]);
		setShowMoveModal(false);
		setSearchTerm("");
		setSearchResults([]);
		setPreviewFile(null);
		setPlayingFile(null);
		setPdfFile(null);
		setPreviewContextFiles([]);
		setPreviewContextIndex(-1);
	}, [activeFolderId]);

	useEffect(() => {
		if (searchTerm.length <= 2) {
			setSearchResults([]);
			return;
		}

		const timer = setTimeout(async () => {
			setIsSearching(true);
			const results = await handleGlobalSearch(searchTerm);
			setSearchResults(results);
			setIsSearching(false);
		}, 500);

		return () => clearTimeout(timer);
	}, [searchTerm]);

	const handleFileClick = (e: React.MouseEvent, id: number) => {
		e.stopPropagation();
		if (e.metaKey || e.ctrlKey) {
			setSelectedIds((ids) =>
				ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id],
			);
		} else {
			setSelectedIds([id]);
		}
	};

	const handlePreview = (file: TelegramFile, orderedFiles?: TelegramFile[]) => {
		const contextFiles = (orderedFiles || displayedFiles).filter(
			(f) => f.type !== "folder",
		);
		const contextIndex = contextFiles.findIndex((f) => f.id === file.id);

		setPreviewContextFiles(contextFiles);
		setPreviewContextIndex(contextIndex);

		const isMedia = isMediaFile(file.name);
		const isPdf = isPdfFile(file.name);

		if (isMedia) {
			setPlayingFile(file);
			setPreviewFile(null);
			setPdfFile(null);
		} else if (isPdf) {
			setPdfFile(file);
			setPreviewFile(null);
			setPlayingFile(null);
		} else {
			setPreviewFile(file);
			setPlayingFile(null);
			setPdfFile(null);
		}
	};

	const navigatePreview = useCallback(
		(step: 1 | -1) => {
			if (previewContextFiles.length === 0) return;

			const currentFileId = previewFile?.id ?? playingFile?.id ?? pdfFile?.id;
			if (!currentFileId) return;

			const currentIndex = previewContextFiles.findIndex(
				(f) => f.id === currentFileId,
			);
			if (currentIndex === -1) return;

			const nextIndex =
				(currentIndex + step + previewContextFiles.length) %
				previewContextFiles.length;
			const nextFile = previewContextFiles[nextIndex];
			if (!nextFile) return;

			setPreviewContextIndex(nextIndex);

			const isMedia = isMediaFile(nextFile.name);
			const isPdf = isPdfFile(nextFile.name);

			if (isMedia) {
				setPlayingFile(nextFile);
				setPreviewFile(null);
				setPdfFile(null);
			} else if (isPdf) {
				setPdfFile(nextFile);
				setPreviewFile(null);
				setPlayingFile(null);
			} else {
				setPreviewFile(nextFile);
				setPlayingFile(null);
				setPdfFile(null);
			}
		},
		[previewContextFiles, previewFile, playingFile, pdfFile],
	);

	const handleNextPreview = useCallback(() => {
		navigatePreview(1);
	}, [navigatePreview]);

	const handlePrevPreview = useCallback(() => {
		navigatePreview(-1);
	}, [navigatePreview]);

	const previewNeighborFiles = useCallback(() => {
		if (previewContextFiles.length === 0) {
			return {
				nextFile: null as TelegramFile | null,
				prevFile: null as TelegramFile | null,
			};
		}

		const currentFileId = previewFile?.id ?? playingFile?.id ?? pdfFile?.id;
		if (!currentFileId) {
			return {
				nextFile: null as TelegramFile | null,
				prevFile: null as TelegramFile | null,
			};
		}

		const currentIdx = previewContextFiles.findIndex(
			(f) => f.id === currentFileId,
		);
		if (currentIdx === -1) {
			return {
				nextFile: null as TelegramFile | null,
				prevFile: null as TelegramFile | null,
			};
		}

		const nextIdx = (currentIdx + 1) % previewContextFiles.length;
		const prevIdx =
			(currentIdx - 1 + previewContextFiles.length) %
			previewContextFiles.length;

		return {
			nextFile: previewContextFiles[nextIdx] || null,
			prevFile: previewContextFiles[prevIdx] || null,
		};
	}, [previewContextFiles, previewFile, playingFile, pdfFile]);

	const handleDropOnFolder = async (
		e: React.DragEvent,
		targetFolderId: number | null,
	) => {
		e.preventDefault();
		e.stopPropagation();

		const dataTransferFileId = e.dataTransfer.getData(
			"application/x-telegram-file-id",
		);

		if (activeFolderId === targetFolderId) return;

		const fileId =
			internalDragRef.current ||
			(dataTransferFileId ? parseInt(dataTransferFileId) : null);

		if (fileId) {
			try {
				const idsToMove = selectedIds.includes(fileId) ? selectedIds : [fileId];

				await invoke("cmd_move_files", {
					messageIds: idsToMove,
					sourceFolderId: activeFolderId,
					targetFolderId: targetFolderId,
				});

				queryClient.invalidateQueries({ queryKey: ["files", activeFolderId] });

				if (selectedIds.includes(fileId)) setSelectedIds([]);

				toast.success(`Moved ${idsToMove.length} file(s).`);

				setInternalDragFileId(null);
			} catch {
				toast.error(`Failed to move file(s).`);
			}
		}
	};

	const currentFolderName =
		activeFolderId === null
			? "Saved Messages"
			: folders.find((f) => f.id === activeFolderId)?.name || "Folder";

	const handleRootDragOver = (e: React.DragEvent) => {
		if (internalDragRef.current) {
			e.preventDefault();
			e.stopPropagation();
			e.dataTransfer.dropEffect = "move";
		}
	};

	const handleRootDragEnter = (e: React.DragEvent) => {
		if (internalDragRef.current) {
			e.preventDefault();
			e.stopPropagation();
			e.dataTransfer.dropEffect = "move";
		}
	};

	const handleGoogleDriveImport = (token: string, items: any[]) => {
		const existingNames = new Set(allFiles.map((f: TelegramFile) => f.name));
		const duplicates = items.filter((item) => existingNames.has(item.name));

		if (duplicates.length > 0) {
			setPendingGoogleDriveImport({ token, items, duplicates });
			return;
		}

		importFiles(token, items);
	};

	const handleSkipExistingGoogleDriveFiles = () => {
		if (!pendingGoogleDriveImport) return;
		const duplicateNames = new Set(
			pendingGoogleDriveImport.duplicates.map((item) => item.name),
		);
		const items = pendingGoogleDriveImport.items.filter(
			(item) => !duplicateNames.has(item.name),
		);
		setPendingGoogleDriveImport(null);
		if (items.length > 0) {
			importFiles(pendingGoogleDriveImport.token, items);
		}
	};

	const handleReplaceExistingGoogleDriveFiles = () => {
		if (!pendingGoogleDriveImport) return;
		const { token, items } = pendingGoogleDriveImport;
		setPendingGoogleDriveImport(null);
		importFiles(token, items);
	};

	const previewNeighbors = previewNeighborFiles();

	return (
		<div
			className="flex h-screen w-full overflow-hidden bg-telegram-bg relative"
			onClick={() => setSelectedIds([])}
			onDragOver={handleRootDragOver}
			onDragEnter={handleRootDragEnter}
		>
			<ExternalDropBlocker onUploadClick={handleManualUpload} />

			<AnimatePresence>
				{showMoveModal && (
					<MoveToFolderModal
						folders={folders}
						onClose={() => setShowMoveModal(false)}
						onSelect={handleBulkMove}
						activeFolderId={activeFolderId}
						key="move-modal"
					/>
				)}
				{showGoogleDriveImportModal && (
					<GoogleDriveImportModal
						onClose={() => setShowGoogleDriveImportModal(false)}
						onImport={handleGoogleDriveImport}
					/>
				)}
				{pendingGoogleDriveImport && (
					<motion.div
						className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
					>
						<motion.div
							className="bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl w-full max-w-md overflow-hidden"
							initial={{ scale: 0.95, opacity: 0 }}
							animate={{ scale: 1, opacity: 1 }}
							exit={{ scale: 0.95, opacity: 0 }}
						>
							<div className="p-4 border-b border-telegram-border">
								<h3 className="text-lg font-medium text-telegram-text">
									Existing Files Found
								</h3>
								<p className="text-sm text-telegram-subtext mt-1">
									{pendingGoogleDriveImport.duplicates.length} file(s) already
									exist in this Telegram folder.
								</p>
							</div>
							<div className="p-4 max-h-64 overflow-y-auto space-y-2">
								{pendingGoogleDriveImport.duplicates.map((item) => (
									<div
										key={item.id}
										className="text-sm text-telegram-subtext bg-telegram-bg border border-telegram-border rounded-lg px-3 py-2 truncate"
										title={item.name}
									>
										{item.name}
									</div>
								))}
							</div>
							<div className="p-4 border-t border-telegram-border bg-telegram-hover flex justify-end gap-2">
								<button
									onClick={() => setPendingGoogleDriveImport(null)}
									className="px-4 py-2 rounded font-medium text-sm text-telegram-subtext hover:bg-telegram-border transition"
								>
									Cancel
								</button>
								<button
									onClick={handleSkipExistingGoogleDriveFiles}
									className="px-4 py-2 rounded font-medium text-sm text-telegram-text bg-telegram-border hover:bg-telegram-border/80 transition"
								>
									Skip Existing
								</button>
								<button
									onClick={handleReplaceExistingGoogleDriveFiles}
									className="px-4 py-2 bg-telegram-primary hover:bg-telegram-primary/90 text-white rounded font-medium text-sm transition"
								>
									Import Anyway
								</button>
							</div>
						</motion.div>
					</motion.div>
				)}
				{playingFile && (
					<MediaPlayer
						file={playingFile}
						onClose={() => setPlayingFile(null)}
						onNext={handleNextPreview}
						onPrev={handlePrevPreview}
						currentIndex={previewContextIndex}
						totalItems={previewContextFiles.length}
						activeFolderId={activeFolderId}
						key="media-player"
					/>
				)}
				{pdfFile && (
					<PdfViewer
						file={pdfFile}
						onClose={() => setPdfFile(null)}
						onNext={handleNextPreview}
						onPrev={handlePrevPreview}
						currentIndex={previewContextIndex}
						totalItems={previewContextFiles.length}
						activeFolderId={activeFolderId}
						key="pdf-viewer"
					/>
				)}
				{isDragging && internalDragFileId === null && (
					<DragDropOverlay key="drag-drop-overlay" />
				)}
			</AnimatePresence>

			<Sidebar
				folders={folders}
				activeFolderId={activeFolderId}
				setActiveFolderId={setActiveFolderId}
				onDrop={handleDropOnFolder}
				onDelete={handleFolderDelete}
				onCreate={handleCreateFolder}
				isSyncing={isSyncing}
				isConnected={isConnected}
				onSync={handleSyncFolders}
				onLogout={handleLogout}
				bandwidth={bandwidth || null}
			/>

			<main
				className="flex-1 flex flex-col"
				onClick={(e) => {
					if (e.target === e.currentTarget) setSelectedIds([]);
				}}
			>
				<TopBar
					currentFolderName={currentFolderName}
					selectedIds={selectedIds}
					onShowMoveModal={() => setShowMoveModal(true)}
					onBulkDownload={handleBulkDownload}
					onBulkDelete={handleBulkDelete}
					onDownloadFolder={handleDownloadFolder}
					viewMode={viewMode}
					setViewMode={setViewMode}
					searchTerm={searchTerm}
					onSearchChange={setSearchTerm}
					onOpenGoogleDriveImport={() => setShowGoogleDriveImportModal(true)}
				/>
				{searchTerm.length > 2 && (
					<div className="px-6 pt-4 pb-0">
						<h2 className="text-sm font-medium text-telegram-subtext">
							Search Results for{" "}
							<span className="text-telegram-primary">"{searchTerm}"</span>
						</h2>
					</div>
				)}
				<FileExplorer
					files={displayedFiles}
					loading={isLoading || isSearching}
					error={error}
					viewMode={viewMode}
					selectedIds={selectedIds}
					activeFolderId={activeFolderId}
					onFileClick={handleFileClick}
					onDelete={handleDelete}
					onDownload={(id, name) => queueDownload(id, name, activeFolderId)}
					onPreview={handlePreview}
					onManualUpload={handleManualUpload}
					onSelectionClear={() => setSelectedIds([])}
					onDrop={handleDropOnFolder}
					onDragStart={(fileId) => setInternalDragFileId(fileId)}
					onDragEnd={() => setTimeout(() => setInternalDragFileId(null), 50)}
				/>
			</main>

			{previewFile && (
				<PreviewModal
					file={previewFile}
					activeFolderId={activeFolderId}
					onClose={() => setPreviewFile(null)}
					onNext={handleNextPreview}
					onPrev={handlePrevPreview}
					currentIndex={previewContextIndex}
					totalItems={previewContextFiles.length}
					nextFile={previewNeighbors.nextFile}
					prevFile={previewNeighbors.prevFile}
				/>
			)}

			<UploadQueue
				items={uploadQueue}
				onClearFinished={() =>
					setUploadQueue((q) =>
						q.filter(
							(i) =>
								i.status !== "success" &&
								i.status !== "error" &&
								i.status !== "cancelled",
						),
					)
				}
				onCancelAll={cancelUploads}
			/>
			<DownloadQueue
				items={downloadQueue}
				onClearFinished={clearDownloads}
				onCancelAll={cancelDownloads}
			/>
			<MigrationQueue
				items={migrationQueue}
				onClearFinished={clearFinishedMigrations}
			/>
		</div>
	);
}
