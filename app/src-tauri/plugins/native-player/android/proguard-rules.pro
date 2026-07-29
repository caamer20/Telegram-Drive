# Tauri discovers plugin commands and activity callbacks by annotation.
-keep @app.tauri.annotation.TauriPlugin class * { *; }
-keepclassmembers class * {
    @app.tauri.annotation.Command <methods>;
    @app.tauri.annotation.ActivityCallback <methods>;
}
-keep @app.tauri.annotation.InvokeArg class * { *; }
