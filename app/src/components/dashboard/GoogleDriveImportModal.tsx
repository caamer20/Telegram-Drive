import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Store } from "@tauri-apps/plugin-store";
import {
	AlertCircle,
	ArrowLeft,
	Check,
	CloudRain,
	Folder,
	LogOut,
	X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { formatBytes } from "../../utils";

export interface GoogleDriveImportModalProps {
	onClose: () => void;
	onImport: (accessToken: string, items: any[]) => void;
}

export function GoogleDriveImportModal({
	onClose,
	onImport,
}: GoogleDriveImportModalProps) {
	const [step, setStep] = useState<"config" | "oauth" | "browse">("config");
	const [clientId, setClientId] = useState("");
	const [clientSecret, setClientSecret] = useState("");
	const [accessToken, setAccessToken] = useState("");

	const [folderStack, setFolderStack] = useState<
		{ id: string; name: string }[]
	>([{ id: "root", name: "My Drive" }]);

	const rootNavigation = [
		{ id: "root", name: "My Drive" },
		{ id: "sharedWithMe", name: "Shared with Me" },
	];

	const [files, setFiles] = useState<any[]>([]);
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState("");

	useEffect(() => {
		Store.load("settings.json").then(async (store) => {
			const savedId = await store.get<string>("gd_client_id");
			const savedSecret = await store.get<string>("gd_client_secret");

			if (savedId) setClientId(savedId);
			if (savedSecret) setClientSecret(savedSecret);

			const savedRefreshToken = await store.get<string>("gd_refresh_token");
			if (savedId && savedSecret && savedRefreshToken) {
				setLoading(true);
				try {
					const res = await invoke<any>("cmd_gd_refresh_token", {
						clientId: savedId,
						clientSecret: savedSecret,
						refreshToken: savedRefreshToken,
					});
					const token = res.accessToken ?? res.access_token;
					if (token) {
						setAccessToken(token);
						await loadFiles(token);
						setStep("browse");
					}
				} catch (e) {
					console.warn("Silent refresh failed:", e);
				} finally {
					setLoading(false);
				}
			}
		});
	}, []);

	const handleConnect = async () => {
		if (!clientId || !clientSecret) {
			setError("Please enter Client ID and Secret");
			return;
		}
		setLoading(true);
		setError("");
		try {
			const store = await Store.load("settings.json");
			await store.set("gd_client_id", clientId);
			await store.set("gd_client_secret", clientSecret);
			await store.save();

			const res = await invoke<any>("cmd_gd_auth_url", { clientId });
			const authUrl = res.authUrl ?? res.auth_url;
			if (!authUrl) {
				throw new Error("Google auth URL missing from backend response");
			}

			setStep("oauth");

			// Start waiting for the redirect BEFORE opening browser
			const codePromise = invoke<any>("cmd_gd_wait_for_auth_code").catch(
				(e) => {
					throw new Error(`Auto-login failed: ${e}`);
				},
			);

			await openUrl(authUrl);

			// Wait for browser callback
			const codeRes = await codePromise;
			const tokenRes = await invoke<any>("cmd_gd_exchange_token", {
				clientId,
				clientSecret,
				code: codeRes.code,
			});

			const token = tokenRes.accessToken ?? tokenRes.access_token;
			const refreshToken = tokenRes.refreshToken ?? tokenRes.refresh_token;
			if (!token)
				throw new Error("Google access token missing from backend response");

			if (refreshToken) {
				await store.set("gd_refresh_token", refreshToken);
				await store.save();
			}

			setAccessToken(token);
			await loadFiles(token);
			setStep("browse");
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	};

	const loadFiles = async (token: string, folderId: string = "root") => {
		setLoading(true);
		try {
			const res = await invoke<any>("cmd_gd_list_files", {
				accessToken: token,
				folderId: folderId,
				pageToken: null,
			});
			setFiles(res.files);
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	};

	const toggleSelect = (id: string) => {
		const next = new Set(selectedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		setSelectedIds(next);
	};

	const toggleSelectAll = () => {
		const importableFiles = files.filter(
			(f) => f.mimeType !== "application/vnd.google-apps.folder",
		);
		if (
			selectedIds.size === importableFiles.length &&
			importableFiles.length > 0
		) {
			setSelectedIds(new Set());
		} else {
			setSelectedIds(new Set(importableFiles.map((f) => f.id)));
		}
	};

	const enterFolder = (id: string, name: string) => {
		let newStack = [...folderStack, { id, name }];
		if (id === "root" || id === "sharedWithMe") {
			newStack = [{ id, name }];
		}
		setFolderStack(newStack);
		setSelectedIds(new Set());
		loadFiles(accessToken, id);
	};

	const navigateBack = () => {
		if (folderStack.length <= 1) return;
		const newStack = folderStack.slice(0, -1);
		setFolderStack(newStack);
		setSelectedIds(new Set());
		loadFiles(accessToken, newStack[newStack.length - 1].id);
	};

	const handleImport = () => {
		const items = files
			.filter((f) => selectedIds.has(f.id))
			.map((f) => ({
				id: f.id,
				name: f.name,
				size: f.size ? parseInt(f.size) : undefined,
			}));
		onImport(accessToken, items);
		onClose();
	};

	const handleLogout = async () => {
		try {
			const store = await Store.load("settings.json");
			await store.delete("gd_refresh_token");
			await store.save();
			setAccessToken("");
			setFiles([]);
			setSelectedIds(new Set());
			setFolderStack([{ id: "root", name: "My Drive" }]);
			setStep("config");
		} catch (e) {
			console.error("Failed to logout:", e);
		}
	};

	return (
		<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in">
			<div className="bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl w-full max-w-2xl flex flex-col max-h-[85vh]">
				<div className="flex justify-between items-center p-4 border-b border-telegram-border">
					<div className="flex items-center gap-2">
						<CloudRain className="w-5 h-5 text-telegram-primary" />
						<h3 className="text-lg font-medium text-telegram-text">
							Import from Google Drive
						</h3>
					</div>
					<div className="flex items-center gap-2">
						{step === "browse" && (
							<button
								onClick={handleLogout}
								title="Disconnect Google Account"
								className="text-telegram-subtext hover:text-red-400 p-1 rounded-md hover:bg-telegram-hover transition mr-2"
							>
								<LogOut className="w-4 h-4" />
							</button>
						)}
						<button
							onClick={onClose}
							className="text-telegram-subtext hover:text-telegram-text p-1 rounded-md hover:bg-telegram-hover transition"
						>
							<X className="w-5 h-5" />
						</button>
					</div>
				</div>

				<div className="p-6 overflow-y-auto">
					{error && (
						<div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-lg flex items-center gap-2 text-sm">
							<AlertCircle className="w-4 h-4 flex-shrink-0" />
							<span>{error}</span>
						</div>
					)}

					{step === "config" && (
						<div className="space-y-4">
							<p className="text-sm text-telegram-subtext">
								To access your Google Drive, you need a Google Cloud OAuth
								Client ID.
							</p>
							<div>
								<label className="block text-xs text-telegram-subtext mb-1">
									Client ID
								</label>
								<input
									type="text"
									value={clientId}
									onChange={(e) => setClientId(e.target.value)}
									className="w-full bg-telegram-bg border border-telegram-border rounded px-3 py-2 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary"
									placeholder="...apps.googleusercontent.com"
								/>
							</div>
							<div>
								<label className="block text-xs text-telegram-subtext mb-1">
									Client Secret
								</label>
								<input
									type="password"
									value={clientSecret}
									onChange={(e) => setClientSecret(e.target.value)}
									className="w-full bg-telegram-bg border border-telegram-border rounded px-3 py-2 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary"
									placeholder="GOCSPX-..."
								/>
							</div>
							<button
								onClick={handleConnect}
								disabled={loading}
								className="w-full bg-telegram-primary hover:bg-telegram-primary/90 text-white font-medium py-2 rounded-lg transition disabled:opacity-50"
							>
								{loading ? "Connecting..." : "Connect to Google Drive"}
							</button>
						</div>
					)}

					{step === "oauth" && (
						<div className="flex flex-col items-center justify-center py-12 text-center">
							<div className="w-16 h-16 border-4 border-telegram-primary/30 border-t-telegram-primary rounded-full animate-spin mb-6"></div>
							<h4 className="text-xl font-medium text-telegram-text mb-2">
								Waiting for Authentication
							</h4>
							<p className="text-telegram-subtext max-w-md">
								Your browser should have opened. Please sign in to Google and
								authorize Telegram Drive. This window will automatically
								continue once complete.
							</p>
						</div>
					)}

					{step === "browse" && (
						<div className="space-y-3">
							<div className="flex items-center gap-2 mb-2">
								{rootNavigation.map((nav) => (
									<button
										key={nav.id}
										onClick={() => enterFolder(nav.id, nav.name)}
										className={`px-3 py-1.5 rounded-full text-xs font-medium transition-colors ${
											folderStack[0]?.id === nav.id
												? "bg-telegram-primary text-white"
												: "bg-telegram-bg text-telegram-subtext hover:text-telegram-text border border-telegram-border"
										}`}
									>
										{nav.name}
									</button>
								))}
							</div>

							<div className="flex items-center justify-between bg-telegram-bg p-2 rounded border border-telegram-border">
								<div className="flex items-center gap-2 overflow-hidden">
									{folderStack.length > 1 && (
										<button
											onClick={navigateBack}
											className="p-1 hover:bg-telegram-border rounded text-telegram-subtext hover:text-telegram-text"
										>
											<ArrowLeft className="w-4 h-4" />
										</button>
									)}
									<span className="text-sm font-medium text-telegram-text truncate">
										{folderStack[folderStack.length - 1].name}
									</span>
								</div>

								{files.some(
									(f) => f.mimeType !== "application/vnd.google-apps.folder",
								) && (
									<button
										onClick={toggleSelectAll}
										className="text-xs font-medium text-telegram-primary hover:bg-telegram-primary/10 px-2 py-1 rounded transition-colors"
									>
										{selectedIds.size > 0 &&
										selectedIds.size ===
											files.filter(
												(f) =>
													f.mimeType !== "application/vnd.google-apps.folder",
											).length
											? "Deselect All"
											: "Select All"}
									</button>
								)}
							</div>

							{loading ? (
								<div className="py-8 text-center text-telegram-subtext text-sm">
									Loading files...
								</div>
							) : files.length === 0 ? (
								<div className="py-8 text-center text-telegram-subtext text-sm">
									No files found in this folder.
								</div>
							) : (
								<div className="space-y-1">
									{files.map((file) => {
										const isSelected = selectedIds.has(file.id);
										const isFolder =
											file.mimeType === "application/vnd.google-apps.folder";
										return (
											<div
												key={file.id}
												onClick={() => {
													if (isFolder) {
														enterFolder(file.id, file.name);
													} else {
														toggleSelect(file.id);
													}
												}}
												className={`flex items-center gap-3 p-2 rounded-lg border cursor-pointer transition-colors ${
													isSelected
														? "bg-telegram-primary/10 border-telegram-primary"
														: isFolder
															? "bg-telegram-bg border-telegram-border hover:border-telegram-primary/50"
															: "bg-telegram-surface border-transparent hover:bg-telegram-hover"
												}`}
											>
												<div
													className={`w-5 h-5 rounded flex items-center justify-center border shrink-0 ${
														isFolder
															? "border-transparent bg-transparent"
															: isSelected
																? "bg-telegram-primary border-telegram-primary text-white"
																: "border-telegram-subtext"
													}`}
												>
													{isFolder ? (
														<Folder
															className="w-5 h-5 text-blue-400"
															fill="currentColor"
															opacity={0.2}
														/>
													) : (
														isSelected && <Check className="w-3 h-3" />
													)}
												</div>
												<div className="flex-1 min-w-0">
													<div className="text-sm font-medium text-telegram-text truncate">
														{file.name}
													</div>
													{!isFolder && file.size && (
														<div className="text-xs text-telegram-subtext">
															{formatBytes(parseInt(file.size))}
														</div>
													)}
												</div>
											</div>
										);
									})}
								</div>
							)}
						</div>
					)}
				</div>

				{step === "browse" && (
					<div className="p-4 border-t border-telegram-border flex justify-between items-center bg-telegram-hover rounded-b-xl">
						<span className="text-sm text-telegram-subtext">
							{selectedIds.size} files selected
						</span>
						<div className="flex gap-2">
							<button
								onClick={onClose}
								className="px-4 py-2 rounded font-medium text-sm text-telegram-subtext hover:bg-telegram-border transition"
							>
								Cancel
							</button>
							<button
								onClick={handleImport}
								disabled={selectedIds.size === 0}
								className="px-4 py-2 bg-telegram-primary hover:bg-telegram-primary/90 text-white rounded font-medium text-sm transition disabled:opacity-50"
							>
								Import Selected
							</button>
						</div>
					</div>
				)}
			</div>
		</div>
	);
}
