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
use desktop::{MediaLibrary, Resolver};
#[cfg(mobile)]
use mobile::{MediaLibrary, Resolver};

pub trait MediaLibraryExt<R: Runtime> {
    fn media_library(&self) -> &MediaLibrary<R>;
}

impl<R: Runtime, T: Manager<R>> MediaLibraryExt<R> for T {
    fn media_library(&self) -> &MediaLibrary<R> {
        self.state::<MediaLibrary<R>>().inner()
    }
}

pub fn init<R, F>(resolver: F) -> TauriPlugin<R>
where
    R: Runtime,
    F: Fn(&AppHandle<R>) -> Result<ResolvedMediaLibrarySession> + Send + Sync + 'static,
{
    let resolver: Resolver<R> = Arc::new(resolver);
    Builder::new("media-library")
        .invoke_handler(tauri::generate_handler![
            commands::open_media_library,
            commands::close_media_library,
            commands::get_media_library_state,
            commands::clear_media_library_data,
        ])
        .setup(move |app, api| {
            #[cfg(mobile)]
            let library = mobile::init(app, api, resolver.clone())?;
            #[cfg(desktop)]
            let library = desktop::init(app, api, resolver.clone())?;
            app.manage(library);
            Ok(())
        })
        .build()
}
