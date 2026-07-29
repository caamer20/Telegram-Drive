use tauri::{command, AppHandle, Runtime};

use crate::{
    NativePlaybackState, NativePlayerExt, NativePlayerResult, NativePlayerSource, Result,
};

#[command]
pub(crate) async fn open_native_player<R: Runtime>(
    app: AppHandle<R>,
    source: NativePlayerSource,
) -> Result<NativePlayerResult> {
    app.native_player().open(source)
}

#[command]
pub(crate) async fn close_native_player<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.native_player().close()
}

#[command]
pub(crate) async fn get_native_playback_state<R: Runtime>(
    app: AppHandle<R>,
) -> Result<NativePlaybackState> {
    app.native_player().state()
}

#[command]
pub(crate) async fn take_pending_native_player_restore<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<NativePlayerSource>> {
    app.native_player().take_pending_restore()
}
