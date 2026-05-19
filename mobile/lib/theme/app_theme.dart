/// Telegram Drive visual theme.
///
/// Matches the React web app's design system. Provides dark (default)
/// and light theme [ThemeData] instances along with the raw color tokens.
library;

import 'package:flutter/material.dart';

// ── Color tokens ─────────────────────────────────────────────────────────

/// Color palette matching the Telegram Drive design system.
///
/// Dark theme values are the defaults. Each token also exposes a
/// light-mode variant for `ThemeData` construction.
abstract final class TelegramColors {
  // ── Dark theme (default) ────────────────────────────────────────────────

  /// Dark background: `#0e1621`
  static const Color bgDark = Color(0xFF0E1621);

  /// Dark surface/card: `#17212b`
  static const Color surfaceDark = Color(0xFF17212B);

  /// Primary accent (amber/gold): `#ffae00`
  static const Color primaryDark = Color(0xFFFFAE00);

  /// Secondary accent (blue): `#2481cc`
  static const Color secondaryDark = Color(0xFF2481CC);

  /// Primary text: `#ffffff`
  static const Color textDark = Color(0xFFFFFFFF);

  /// Secondary/subtle text: `#8e9fb3`
  static const Color subtextDark = Color(0xFF8E9FB3);

  /// Border color: `rgba(255, 255, 255, 0.1)`
  static const Color borderDark = Color(0x1AFFFFFF);

  /// Hover/overlay: `rgba(255, 255, 255, 0.05)`
  static const Color hoverDark = Color(0x0DFFFFFF);

  /// Scaffold background (dark).
  static const Color scaffoldBackgroundDark = bgDark;

  // ── Light theme overrides ───────────────────────────────────────────────

  /// Light background: `#f0f2f5`
  static const Color bgLight = Color(0xFFF0F2F5);

  /// Light surface/card: `#ffffff`
  static const Color surfaceLight = Color(0xFFFFFFFF);

  /// Primary accent (amber/gold, light): `#e69500`
  static const Color primaryLight = Color(0xFFE69500);

  /// Secondary accent (blue, light): `#2481cc`
  static const Color secondaryLight = Color(0xFF2481CC);

  /// Primary text (light): `#1a1a1a`
  static const Color textLight = Color(0xFF1A1A1A);

  /// Secondary text (light): `#65676b`
  static const Color subtextLight = Color(0xFF65676B);

  /// Border color (light): `rgba(0, 0, 0, 0.1)`
  static const Color borderLight = Color(0x1A000000);

  /// Hover/overlay (light): `rgba(0, 0, 0, 0.03)`
  static const Color hoverLight = Color(0x08000000);

  /// Scaffold background (light).
  static const Color scaffoldBackgroundLight = bgLight;
}

// ── Theme definitions ────────────────────────────────────────────────────

/// Pre-built [ThemeData] instances for Telegram Drive.
///
/// Usage:
/// ```dart
/// MaterialApp(
///   theme: AppTheme.darkTheme,
///   darkTheme: AppTheme.darkTheme,
///   themeMode: ThemeMode.dark,
/// )
/// ```
abstract final class AppTheme {
  /// Shared font family stack matching the React app.
  static const String _fontFamily = 'Inter';

  /// The default dark theme.
  static ThemeData get darkTheme => _buildTheme(
        brightness: Brightness.dark,
        primary: TelegramColors.primaryDark,
        secondary: TelegramColors.secondaryDark,
        background: TelegramColors.bgDark,
        surface: TelegramColors.surfaceDark,
        text: TelegramColors.textDark,
        subtext: TelegramColors.subtextDark,
        border: TelegramColors.borderDark,
        hover: TelegramColors.hoverDark,
        scaffoldBg: TelegramColors.scaffoldBackgroundDark,
      );

  /// The light theme.
  static ThemeData get lightTheme => _buildTheme(
        brightness: Brightness.light,
        primary: TelegramColors.primaryLight,
        secondary: TelegramColors.secondaryLight,
        background: TelegramColors.bgLight,
        surface: TelegramColors.surfaceLight,
        text: TelegramColors.textLight,
        subtext: TelegramColors.subtextLight,
        border: TelegramColors.borderLight,
        hover: TelegramColors.hoverLight,
        scaffoldBg: TelegramColors.scaffoldBackgroundLight,
      );

  // ── Theme builder ──────────────────────────────────────────────────────

  static ThemeData _buildTheme({
    required Brightness brightness,
    required Color primary,
    required Color secondary,
    required Color background,
    required Color surface,
    required Color text,
    required Color subtext,
    required Color border,
    required Color hover,
    required Color scaffoldBg,
  }) {
    final isDark = brightness == Brightness.dark;
    final colorScheme = ColorScheme(
      brightness: brightness,
      primary: primary,
      onPrimary: text,
      secondary: secondary,
      onSecondary: text,
      surface: surface,
      onSurface: text,
      error: const Color(0xFFE53935),
      onError: Colors.white,
    );

    return ThemeData(
      useMaterial3: true,
      fontFamily: _fontFamily,
      colorScheme: colorScheme,
      scaffoldBackgroundColor: scaffoldBg,

      // ── AppBar ────────────────────────────────────────────────────────
      appBarTheme: AppBarTheme(
        backgroundColor: surface,
        foregroundColor: text,
        elevation: 0,
        centerTitle: false,
        titleTextStyle: TextStyle(
          fontFamily: _fontFamily,
          fontSize: 18,
          fontWeight: FontWeight.w600,
          color: text,
        ),
        iconTheme: IconThemeData(color: text),
      ),

      // ── Cards ─────────────────────────────────────────────────────────
      cardTheme: CardThemeData(
        color: surface,
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(12),
          side: BorderSide(color: border),
        ),
        clipBehavior: Clip.antiAlias,
        margin: const EdgeInsets.all(4),
      ),

      // ── Input decoration ──────────────────────────────────────────────
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: isDark ? const Color(0xFF0E1621) : Colors.white,
        hintStyle: TextStyle(color: subtext),
        labelStyle: TextStyle(color: subtext),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: border),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: border),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: primary, width: 1.5),
        ),
        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      ),

      // ── Elevated buttons ──────────────────────────────────────────────
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          backgroundColor: primary,
          foregroundColor: isDark ? Colors.black : Colors.white,
          elevation: 0,
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 14),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(10),
          ),
          textStyle: TextStyle(
            fontFamily: _fontFamily,
            fontSize: 15,
            fontWeight: FontWeight.w600,
          ),
        ),
      ),

      // ── Text button ───────────────────────────────────────────────────
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          foregroundColor: primary,
          textStyle: TextStyle(
            fontFamily: _fontFamily,
            fontSize: 14,
            fontWeight: FontWeight.w500,
          ),
        ),
      ),

      // ── Bottom sheet ──────────────────────────────────────────────────
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: surface,
        surfaceTintColor: surface,
        shape: RoundedRectangleBorder(
          borderRadius: const BorderRadius.vertical(top: Radius.circular(20)),
          side: BorderSide(color: border),
        ),
      ),

      // ── Dialog ────────────────────────────────────────────────────────
      dialogTheme: DialogThemeData(
        backgroundColor: surface,
        surfaceTintColor: surface,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(16),
        ),
      ),

      // ── Floating action button ────────────────────────────────────────
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: primary,
        foregroundColor: isDark ? Colors.black : Colors.white,
        elevation: 4,
      ),

      // ── Icon theme ────────────────────────────────────────────────────
      iconTheme: IconThemeData(color: subtext, size: 24),

      // ── Divider ───────────────────────────────────────────────────────
      dividerTheme: DividerThemeData(
        color: border,
        thickness: 0.5,
        space: 0,
      ),

      // ── Chip ──────────────────────────────────────────────────────────
      chipTheme: ChipThemeData(
        backgroundColor: isDark ? const Color(0xFF1E2D3D) : const Color(0xFFE8E8E8),
        labelStyle: TextStyle(color: text, fontSize: 13),
        side: BorderSide.none,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(8),
        ),
      ),

      // ── List tile ─────────────────────────────────────────────────────
      listTileTheme: ListTileThemeData(
        textColor: text,
        iconColor: subtext,
        subtitleTextStyle: TextStyle(color: subtext, fontSize: 13),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
      ),

      // ── Text selection ────────────────────────────────────────────────
      textSelectionTheme: TextSelectionThemeData(
        cursorColor: primary,
        selectionColor: primary.withValues(alpha: 0.3),
        selectionHandleColor: primary,
      ),

      // ── Navigation bar (bottom nav) ───────────────────────────────────
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: surface,
        indicatorColor: primary.withValues(alpha: 0.15),
        labelTextStyle: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.selected)) {
            return TextStyle(color: primary, fontSize: 12, fontWeight: FontWeight.w600);
          }
          return TextStyle(color: subtext, fontSize: 12);
        }),
        iconTheme: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.selected)) {
            return IconThemeData(color: primary, size: 24);
          }
          return IconThemeData(color: subtext, size: 24);
        }),
      ),

      // ── Progress indicator ────────────────────────────────────────────
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: primary,
        linearTrackColor: border,
        circularTrackColor: border,
      ),

      // ── Snack bar ─────────────────────────────────────────────────────
      snackBarTheme: SnackBarThemeData(
        backgroundColor: surface,
        contentTextStyle: TextStyle(color: text, fontSize: 14),
        behavior: SnackBarBehavior.floating,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(10),
          side: BorderSide(color: border),
        ),
      ),
    );
  }
}
