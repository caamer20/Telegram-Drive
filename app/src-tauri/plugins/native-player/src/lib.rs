use std::sync::Arc;
use tauri::{
    plugin::{Builder, TauriPlugin},
    AppHandle, Manager, Runtime,
};

pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::{NativePlayer, Resolver};
#[cfg(mobile)]
use mobile::{NativePlayer, Resolver};

pub trait NativePlayerExt<R: Runtime> {
    fn native_player(&self) -> &NativePlayer<R>;
}

impl<R: Runtime, T: Manager<R>> NativePlayerExt<R> for T {
    fn native_player(&self) -> &NativePlayer<R> {
        self.state::<NativePlayer<R>>().inner()
    }
}

pub fn init<R, F>(resolver: F) -> TauriPlugin<R>
where
    R: Runtime,
    F: Fn(&AppHandle<R>, &NativePlayerSource) -> Result<ResolvedStreamSource>
        + Send
        + Sync
        + 'static,
{
    let resolver: Resolver<R> = Arc::new(resolver);
    Builder::new("native-player")
        .invoke_handler(tauri::generate_handler![
            commands::open_native_player,
            commands::close_native_player,
            commands::get_native_playback_state,
            commands::take_pending_native_player_restore,
        ])
        .setup(move |app, api| {
            #[cfg(mobile)]
            let native_player = mobile::init(app, api, resolver.clone())?;
            #[cfg(desktop)]
            let native_player = desktop::init(app, api, resolver.clone())?;
            app.manage(native_player);
            Ok(())
        })
        .build()
}
