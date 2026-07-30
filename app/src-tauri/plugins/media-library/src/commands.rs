use tauri::{command, AppHandle, Runtime};

use crate::{MediaLibraryExt, MediaLibraryResult, MediaLibraryState, Result};

#[command]
pub(crate) async fn open_media_library<R: Runtime>(
    app: AppHandle<R>,
) -> Result<MediaLibraryResult> {
    app.media_library().open().await
}

#[command]
pub(crate) async fn close_media_library<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.media_library().close().await
}

#[command]
pub(crate) async fn get_media_library_state<R: Runtime>(
    app: AppHandle<R>,
) -> Result<MediaLibraryState> {
    app.media_library().state().await
}

#[command]
pub(crate) async fn clear_media_library_data<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.media_library().clear_data().await
}
