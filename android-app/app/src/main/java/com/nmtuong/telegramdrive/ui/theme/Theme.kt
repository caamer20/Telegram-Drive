package com.nmtuong.telegramdrive.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val Light = lightColorScheme(primary = Color(0xFF006B5F), secondary = Color(0xFF4A635E))
private val Dark = darkColorScheme(primary = Color(0xFF53DBC6), secondary = Color(0xFFB1CCC5))

@Composable fun TelegramDriveTheme(content: @Composable () -> Unit) {
  MaterialTheme(colorScheme = if (isSystemInDarkTheme()) Dark else Light, content = content)
}
