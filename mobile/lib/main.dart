import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import 'models/api_models.dart';
import 'providers/connection_provider.dart';
import 'providers/files_provider.dart';
import 'providers/folders_provider.dart';
import 'providers/media_provider.dart';
import 'screens/connect_screen.dart';
import 'screens/dashboard_screen.dart';
import 'screens/media_player_screen.dart';
import 'screens/pdf_viewer_screen.dart';
import 'services/api_service.dart';
import 'theme/app_theme.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const TelegramDriveApp());
}

/// Root widget that wires together all providers and screens via
/// [MultiProvider] and named-route navigation.
class TelegramDriveApp extends StatelessWidget {
  const TelegramDriveApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        // ── Connection state (server URL, API key, health check) ──────────
        ChangeNotifierProvider(create: (_) => ConnectionProvider()),

        // ── Folders — rebuilt when the API client becomes available ────────
        ChangeNotifierProxyProvider<ConnectionProvider, FoldersProvider>(
          create: (_) =>
              FoldersProvider(TelegramDriveApi(baseUrl: '', apiKey: '')),
          update: (_, connection, previous) {
            if (connection.api != null) {
              return FoldersProvider(connection.api!);
            }
            return previous!;
          },
        ),

        // ── Files — rebuilt when the API client becomes available ──────────
        ChangeNotifierProxyProvider<ConnectionProvider, FilesProvider>(
          create: (_) =>
              FilesProvider(TelegramDriveApi(baseUrl: '', apiKey: '')),
          update: (_, connection, previous) {
            if (connection.api != null) {
              return FilesProvider(connection.api!);
            }
            return previous!;
          },
        ),

        // ── Media playback state (playlist, shuffle, repeat) ───────────────
        ChangeNotifierProvider(create: (_) => MediaProvider()),
      ],
      child: MaterialApp(
        title: 'Telegram Drive',
        theme: AppTheme.darkTheme,
        themeMode: ThemeMode.dark,
        home: const _AppShell(),
        onGenerateRoute: _onGenerateRoute,
      ),
    );
  }

  /// Named-route generator for screens that need provider access.
  static Route<dynamic>? _onGenerateRoute(RouteSettings settings) {
    switch (settings.name) {
      case '/media-player':
        final args = settings.arguments as Map<String, dynamic>;
        final file = args['file'] as TelegramFile;
        return MaterialPageRoute(
          builder: (_) => _MediaPlayerGate(file: file),
        );
      case '/pdf-viewer':
        final args = settings.arguments as Map<String, dynamic>;
        final file = args['file'] as TelegramFile;
        return MaterialPageRoute(
          builder: (_) => _PdfViewerGate(file: file),
        );
      default:
        return MaterialPageRoute(
          builder: (_) => const _AppShell(),
        );
    }
  }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Internal routing widgets
// ═══════════════════════════════════════════════════════════════════════════════

/// Gates between [ConnectScreen] and [DashboardScreen] based on
/// [ConnectionProvider.isConnected].
///
/// On first launch, triggers automatic reconnection from saved credentials.
/// Once connected, the widget tree switches to [DashboardScreen] without
/// a Navigator transition — the consumer rebuild handles it.
class _AppShell extends StatefulWidget {
  const _AppShell();

  @override
  State<_AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<_AppShell> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<ConnectionProvider>().loadSavedConnection();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<ConnectionProvider>(
      builder: (context, connection, _) {
        if (connection.isConnected) {
          return DashboardScreen(
            onPlayMedia: (file, playlist) {
              Navigator.pushNamed(
                context,
                '/media-player',
                arguments: {'file': file},
              );
            },
          );
        }
        return ConnectScreen(
          connectionProvider: connection,
          onConnected: () {
            // Navigation is handled by the Consumer rebuilding
            // when ConnectionProvider.isConnected becomes true.
          },
        );
      },
    );
  }
}

/// Extracts context from providers and builds a [MediaPlayerScreen]
/// for the tapped [file].
///
/// Sets the current media on [MediaProvider] before opening the screen,
/// so the player has the correct initial file.
class _MediaPlayerGate extends StatefulWidget {
  final TelegramFile file;

  const _MediaPlayerGate({required this.file});

  @override
  State<_MediaPlayerGate> createState() => _MediaPlayerGateState();
}

class _MediaPlayerGateState extends State<_MediaPlayerGate> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final media = context.read<MediaProvider>();

      // If the same file is already playing (e.g. from mini-player tap),
      // just show the screen without re-initializing.
      if (widget.file == media.currentFile) return;

      final files = context.read<FilesProvider>();
      final connection = context.read<ConnectionProvider>();
      final mediaFiles = files.displayedFiles.where((f) => f.isMedia).toList();
      media.playFile(
        widget.file,
        playlist: mediaFiles,
        api: connection.api,
      );
    });
  }

  @override
  Widget build(BuildContext context) {
    final connection = context.watch<ConnectionProvider>();

    final api = connection.api;
    if (api == null) return const SizedBox();

    return MediaPlayerScreen(
      mediaProvider: context.read<MediaProvider>(),
      api: api,
    );
  }
}

/// Extracts [api] from [ConnectionProvider] and builds a [PdfViewerScreen]
/// for the tapped [file].
class _PdfViewerGate extends StatefulWidget {
  final TelegramFile file;

  const _PdfViewerGate({required this.file});

  @override
  State<_PdfViewerGate> createState() => _PdfViewerGateState();
}

class _PdfViewerGateState extends State<_PdfViewerGate> {
  @override
  Widget build(BuildContext context) {
    final connection = context.watch<ConnectionProvider>();

    final api = connection.api;
    if (api == null) return const SizedBox();

    return PdfViewerScreen(file: widget.file, api: api);
  }
}

