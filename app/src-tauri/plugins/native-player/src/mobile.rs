use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::{
    NativePlaybackState, NativePlayerRequest, NativePlayerResult, NativePlayerSource,
    ResolvedStreamSource, Result,
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
    pub fn open(&self, source: NativePlayerSource) -> Result<NativePlayerResult> {
        source.validate()?;
        self.is_open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| crate::Error::AlreadyOpen)?;

        let result = (|| {
            let resolved = (self.resolver)(&self.app, &source)?;
            resolved.validate_loopback()?;
            let request = NativePlayerRequest::new(source, resolved);
            self.handle
                .run_mobile_plugin("openNativePlayer", request)
                .map_err(Into::into)
        })();
        self.is_open.store(false, Ordering::Release);
        result
    }

    pub fn close(&self) -> Result<()> {
        self.handle
            .run_mobile_plugin::<()>("closeNativePlayer", ())
            .map_err(Into::into)
    }

    pub fn state(&self) -> Result<NativePlaybackState> {
        self.handle
            .run_mobile_plugin("getNativePlaybackState", ())
            .map_err(Into::into)
    }

    pub fn take_pending_restore(&self) -> Result<Option<NativePlayerSource>> {
        self.handle
            .run_mobile_plugin("takePendingRestore", ())
            .map_err(Into::into)
    }
}
