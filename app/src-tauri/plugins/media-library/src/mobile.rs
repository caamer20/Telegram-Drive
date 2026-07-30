use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::{
    ClearMediaLibraryRequest, MediaLibraryResult, MediaLibraryState, OpenMediaLibraryRequest,
    ResolvedMediaLibrarySession, Result,
};

const PLUGIN_IDENTIFIER: &str = "com.cameronamer.telegramdrive.medialibrary";

pub(crate) type Resolver<R> =
    Arc<dyn Fn(&AppHandle<R>) -> Result<ResolvedMediaLibrarySession> + Send + Sync + 'static>;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    api: PluginApi<R, C>,
    resolver: Resolver<R>,
) -> Result<MediaLibrary<R>> {
    #[cfg(target_os = "ios")]
    return Err(crate::Error::UnsupportedPlatform);
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "MediaLibraryPlugin")?;
    Ok(MediaLibrary {
        app: app.clone(),
        handle,
        resolver,
        is_open: AtomicBool::new(false),
    })
}

pub struct MediaLibrary<R: Runtime> {
    app: AppHandle<R>,
    handle: PluginHandle<R>,
    resolver: Resolver<R>,
    is_open: AtomicBool,
}

struct OpenGuard<'a>(&'a AtomicBool);

impl<'a> OpenGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| crate::Error::AlreadyOpen)?;
        Ok(Self(flag))
    }
}

impl Drop for OpenGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl<R: Runtime> MediaLibrary<R> {
    pub async fn open(&self) -> Result<MediaLibraryResult> {
        let _guard = OpenGuard::acquire(&self.is_open)?;
        let session = (self.resolver)(&self.app)?;
        session.validate()?;
        self.handle
            .run_mobile_plugin_async("openMediaLibrary", OpenMediaLibraryRequest::from(session))
            .await
            .map_err(Into::into)
    }

    pub async fn close(&self) -> Result<()> {
        self.handle
            .run_mobile_plugin_async::<()>("closeMediaLibrary", ())
            .await
            .map_err(Into::into)
    }

    pub async fn state(&self) -> Result<MediaLibraryState> {
        self.handle
            .run_mobile_plugin_async("getMediaLibraryState", ())
            .await
            .map_err(Into::into)
    }

    pub async fn clear_data(&self) -> Result<()> {
        self.clear_account(None).await
    }

    pub async fn clear_account(&self, account_id: Option<i64>) -> Result<()> {
        self.handle
            .run_mobile_plugin_async::<()>(
                "clearMediaLibraryData",
                ClearMediaLibraryRequest { account_id },
            )
            .await
            .map_err(Into::into)
    }
}
