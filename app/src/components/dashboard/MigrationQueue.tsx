import { AlertCircle, Check, CloudRain, X } from "lucide-react";
import type { MigrationItem } from "../../hooks/useGoogleDriveMigration";

interface MigrationQueueProps {
	items: MigrationItem[];
	onClearFinished: () => void;
}

export function MigrationQueue({
	items,
	onClearFinished,
}: MigrationQueueProps) {
	if (items.length === 0) return null;

	const activeCount = items.filter(
		(i) => i.status !== "done" && i.status !== "error",
	).length;
	const completedCount = items.filter((i) => i.status === "done").length;

	return (
		<div className="fixed bottom-4 left-4 w-96 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl overflow-hidden z-[100]">
			<div className="p-3 border-b border-telegram-border bg-telegram-hover flex justify-between items-center">
				<div className="flex items-center gap-2">
					<CloudRain className="w-4 h-4 text-telegram-primary" />
					<h4 className="text-sm font-medium text-telegram-text">
						Google Drive Import
					</h4>
					{activeCount > 0 && (
						<span className="text-xs px-1.5 py-0.5 bg-telegram-primary/20 text-telegram-primary rounded-full">
							{activeCount} active
						</span>
					)}
				</div>
				<div className="flex gap-2">
					{completedCount > 0 && (
						<button
							onClick={onClearFinished}
							className="text-xs text-telegram-primary hover:text-telegram-text transition-colors"
						>
							Clear Finished
						</button>
					)}
				</div>
			</div>
			<div className="max-h-60 overflow-y-auto p-2 space-y-2">
				{items.map((item) => (
					<div
						key={item.id}
						className="flex flex-col gap-1 p-2 bg-telegram-hover rounded"
					>
						<div className="flex items-center gap-3 text-sm">
							<div className="flex-shrink-0">
								{item.status === "starting" && (
									<div className="w-4 h-4 rounded-full border-2 border-yellow-500 border-t-transparent animate-spin" />
								)}
								{item.status === "downloading" && (
									<div className="w-4 h-4 rounded-full border-2 border-blue-500 border-t-transparent animate-spin" />
								)}
								{item.status === "uploading" && (
									<div className="w-4 h-4 rounded-full border-2 border-telegram-primary border-t-transparent animate-spin" />
								)}
								{item.status === "done" && (
									<div className="w-4 h-4 rounded-full bg-green-500/20 flex items-center justify-center">
										<Check className="w-3 h-3 text-green-500" />
									</div>
								)}
								{item.status === "error" && (
									<div className="w-4 h-4 rounded-full bg-red-500/20 flex items-center justify-center">
										<X className="w-3 h-3 text-red-500" />
									</div>
								)}
							</div>
							<div
								className="flex-1 truncate text-telegram-subtext"
								title={item.filename}
							>
								{item.filename}
							</div>
							{item.status !== "done" && item.status !== "error" && (
								<div className="text-xs text-telegram-primary font-mono">
									{item.progress}%
								</div>
							)}
						</div>
						{item.status !== "done" && item.status !== "error" && (
							<div className="w-full bg-telegram-border h-1 mt-1 rounded-full overflow-hidden">
								<div
									className="bg-telegram-primary h-full rounded-full transition-all duration-300"
									style={{ width: `${item.progress}%` }}
								/>
							</div>
						)}
						{item.status === "error" && item.error && (
							<div className="flex items-center gap-1 text-xs text-red-400 mt-1">
								<AlertCircle className="w-3 h-3" />
								<span className="truncate">{item.error}</span>
							</div>
						)}
						<div className="text-[10px] text-telegram-subtext px-7">
							{item.status === "downloading" && "Fetching from Google Drive..."}
							{item.status === "uploading" && "Uploading to Telegram..."}
						</div>
					</div>
				))}
			</div>
		</div>
	);
}
