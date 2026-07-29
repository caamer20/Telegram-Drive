const COMMANDS: &[&str] = &[
    "open_native_player",
    "close_native_player",
    "get_native_playback_state",
    "take_pending_native_player_restore",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
