const COMMANDS: &[&str] = &[
    "open_media_library",
    "close_media_library",
    "get_media_library_state",
    "clear_media_library_data",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
