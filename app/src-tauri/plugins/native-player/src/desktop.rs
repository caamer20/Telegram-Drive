use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::{NativePlaybackState, NativePlayerResult, NativePlayerSource, Result};

pub(crate) type Resolver<R> = std::sync::Arc<
    dyn Fn(&AppHandle<R>, &NativePlayerSource) -> Result<crate::ResolvedStreamSource>
        + Send
        + Sync
        + 'static,
>;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
    _resolver: Resolver<R>,
) -> Result<NativePlayer<R>> {
    Ok(NativePlayer(app.clone()))
}

pub struct NativePlayer<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

impl<R: Runtime> NativePlayer<R> {
    pub async fn open(&self, _source: NativePlayerSource) -> Result<NativePlayerResult> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub async fn close(&self) -> Result<()> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub async fn state(&self) -> Result<NativePlaybackState> {
        Ok(NativePlaybackState::default())
    }

    pub async fn take_pending_restore(&self) -> Result<Option<NativePlayerSource>> {
        Ok(None)
    }

    pub async fn clear_pending_restore(&self) -> Result<()> {
        Ok(())
    }
}
