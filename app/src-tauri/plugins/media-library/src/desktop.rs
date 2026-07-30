use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::{MediaLibraryResult, MediaLibraryState, Result};

pub(crate) type Resolver<R> = std::sync::Arc<
    dyn Fn(&AppHandle<R>) -> Result<crate::ResolvedMediaLibrarySession> + Send + Sync + 'static,
>;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
    _resolver: Resolver<R>,
) -> Result<MediaLibrary<R>> {
    Ok(MediaLibrary(app.clone()))
}

pub struct MediaLibrary<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

impl<R: Runtime> MediaLibrary<R> {
    pub async fn open(&self) -> Result<MediaLibraryResult> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub async fn close(&self) -> Result<()> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub async fn state(&self) -> Result<MediaLibraryState> {
        Ok(MediaLibraryState::default())
    }

    pub async fn clear_data(&self) -> Result<()> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub async fn clear_account(&self, _account_id: Option<i64>) -> Result<()> {
        Err(crate::Error::UnsupportedPlatform)
    }
}
