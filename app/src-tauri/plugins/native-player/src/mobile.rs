use serde::de::DeserializeOwned;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::{
    NativePlaybackState, NativePlayerResult, NativePlayerSource, ResolvedStreamSource, Result,
};

const PLUGIN_IDENTIFIER: &str = "com.cameronamer.telegramdrive.nativeplayer";

pub(crate) type Resolver<R> = Arc<
    dyn Fn(&AppHandle<R>, &NativePlayerSource) -> Result<ResolvedStreamSource>
        + Send
        + Sync
        + 'static,
>;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    api: PluginApi<R, C>,
    resolver: Resolver<R>,
) -> Result<NativePlayer<R>> {
    #[cfg(target_os = "ios")]
    return Err(crate::Error::UnsupportedPlatform);
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "NativePlayerPlugin")?;
    Ok(NativePlayer {
        app: app.clone(),
        handle,
        resolver,
        is_open: AtomicBool::new(false),
    })
}

pub struct NativePlayer<R: Runtime> {
    app: AppHandle<R>,
    handle: PluginHandle<R>,
    resolver: Resolver<R>,
    is_open: AtomicBool,
}

impl<R: Runtime> NativePlayer<R> {
    pub async fn open(&self, source: NativePlayerSource) -> Result<NativePlayerResult> {
        crate::open::run_guarded_open(
            &self.is_open,
            source,
            |source| (self.resolver)(&self.app, source),
            |request| async move {
                self.handle
                    .run_mobile_plugin_async("openNativePlayer", request)
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    pub async fn close(&self) -> Result<()> {
        self.handle
            .run_mobile_plugin_async::<()>("closeNativePlayer", ())
            .await
            .map_err(Into::into)
    }

    pub async fn state(&self) -> Result<NativePlaybackState> {
        self.handle
            .run_mobile_plugin_async("getNativePlaybackState", ())
            .await
            .map_err(Into::into)
    }

    pub async fn take_pending_restore(&self) -> Result<Option<NativePlayerSource>> {
        self.handle
            .run_mobile_plugin_async("takePendingRestore", ())
            .await
            .map_err(Into::into)
    }

    pub async fn clear_pending_restore(&self) -> Result<()> {
        self.handle
            .run_mobile_plugin_async::<()>("clearPendingRestore", ())
            .await
            .map_err(Into::into)
    }
}
