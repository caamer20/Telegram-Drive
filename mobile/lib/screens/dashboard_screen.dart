import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../models/api_models.dart';
import '../providers/connection_provider.dart';
import '../providers/files_provider.dart';
import '../providers/folders_provider.dart';
import '../providers/media_provider.dart';
import '../theme/app_theme.dart';

// =====================================================================
// Constants
// =====================================================================

const double _tabletBreakpoint = 600.0;
const double _sidebarWidth = 250.0;

// =====================================================================
// File-type helpers
// =====================================================================

IconData _fileTypeIcon(TelegramFile file) {
  if (file.isVideo) return Icons.play_circle_outlined;
  if (file.isAudio) return Icons.music_note_outlined;
  if (file.isImage) return Icons.photo_outlined;
  if (file.isPdf) return Icons.description_outlined;
  return Icons.insert_drive_file;
}

Color _fileTypeColor(TelegramFile file) {
  if (file.isImage) return const Color(0xFFE91E63);
  if (file.isVideo) return const Color(0xFF9C27B0);
  if (file.isAudio) return const Color(0xFF4CAF50);
  if (file.isPdf) return const Color(0xFFF44336);
  return const Color(0xFF9E9E9E);
}

// =====================================================================
// Helpers
// =====================================================================

String _formatDate(String iso) {
  try {
    final date = DateTime.parse(iso);
    const months = [
      'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
      'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
    ];
    return '${months[date.month - 1]} ${date.day}, ${date.year}';
  } catch (_) {
    return iso;
  }
}

// =====================================================================
// DashboardScreen
// =====================================================================

/// Primary screen after connecting to Telegram Drive.
///
/// Shows a folder sidebar (persistent on tablet, drawer on phone)
/// and a file grid/list in the main content area.
///
/// Requires [FoldersProvider], [FilesProvider], and [ConnectionProvider]
/// to be available in the widget tree (via Provider).
class DashboardScreen extends StatefulWidget {
  /// Called when the user taps a playable media file.
  ///
  /// Receives the tapped file and a playlist of all currently displayed
  /// media files for seamless next/previous navigation.
  final void Function(TelegramFile file, List<TelegramFile> playlist)?
      onPlayMedia;

  const DashboardScreen({super.key, this.onPlayMedia});

  @override
  State<DashboardScreen> createState() => _DashboardScreenState();
}

class _DashboardScreenState extends State<DashboardScreen> {
  _ViewMode _viewMode = _ViewMode.grid;
  final TextEditingController _searchCtrl = TextEditingController();
  final ScrollController _gridScrollCtrl = ScrollController();
  final GlobalKey<ScaffoldState> _scaffoldKey = GlobalKey<ScaffoldState>();
  Timer? _debounceTimer;

  // ── Lifecycle ────────────────────────────────────────────────────

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _loadData());
  }

  @override
  void dispose() {
    _searchCtrl.dispose();
    _gridScrollCtrl.dispose();
    _debounceTimer?.cancel();
    super.dispose();
  }

  void _loadData() {
    context.read<FoldersProvider>().loadFolders();
    context.read<FilesProvider>().loadFiles(null);
  }

  void _onFolderSelected(int? folderId) {
    final folders = context.read<FoldersProvider>();
    final files = context.read<FilesProvider>();
    folders.setActiveFolder(folderId);
    files.loadFiles(folderId);
    _searchCtrl.clear();
    files.clearSearch();
    // Close drawer on phone
    if (_scaffoldKey.currentState?.isDrawerOpen ?? false) {
      Navigator.of(context).pop();
    }
  }

  void _onSearchChanged(String query) {
    _debounceTimer?.cancel();
    _debounceTimer = Timer(const Duration(milliseconds: 300), () async {
      final files = context.read<FilesProvider>();
      final folders = context.read<FoldersProvider>();
      if (query.isEmpty) {
        files.clearSearch();
      } else {
        await files.search(query, folderId: folders.activeFolderId);
        if (files.searchError != null && mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text(files.searchError!)),
          );
        }
      }
    });
  }

  void _onFileTap(TelegramFile file) {
    if (file.isMedia) {
      final files = context.read<FilesProvider>();
      final playlist =
          files.displayedFiles.where((f) => f.isMedia).toList();
      widget.onPlayMedia?.call(file, playlist);
    } else if (file.isPdf) {
      Navigator.pushNamed(
        context,
        '/pdf-viewer',
        arguments: {'file': file},
      );
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('${file.name} — ${file.formatBytes()}')),
      );
    }
  }

  Future<void> _onRefresh() async {
    final folders = context.read<FoldersProvider>();
    final files = context.read<FilesProvider>();
    await Future.wait([
      folders.loadFolders(),
      files.loadFiles(folders.activeFolderId),
    ]);
  }

  void _onDisconnect() async {
    await context.read<ConnectionProvider>().disconnect();
    if (mounted) {
      Navigator.of(context).pushNamedAndRemoveUntil('/', (route) => false);
    }
  }

  // ── Build ────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final folders = context.watch<FoldersProvider>();
    final files = context.watch<FilesProvider>();

    return LayoutBuilder(
      builder: (context, constraints) {
        final isWide = constraints.maxWidth > _tabletBreakpoint;

        return Scaffold(
          key: _scaffoldKey,
          backgroundColor: TelegramColors.bgDark,

          // On phone the sidebar lives in a drawer
          drawer: isWide
              ? null
              : Drawer(child: _buildSidebar(folders)),

          body: SafeArea(
            child: isWide
                ? Row(
                    children: [
                      SizedBox(
                        width: _sidebarWidth,
                        child: _buildSidebar(folders),
                      ),
                      const VerticalDivider(width: 1),
                      Expanded(child: _buildMainContent(files, folders)),
                    ],
                  )
                : _buildMainContent(files, folders),
          ),
        );
      },
    );
  }

  // ── Sidebar ──────────────────────────────────────────────────────

  Widget _buildSidebar(FoldersProvider folders) {
    return Container(
      color: TelegramColors.surfaceDark,
      child: Column(
        children: [
          // Header
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 20, 16, 8),
            child: Row(
              children: [
                Icon(Icons.cloud, color: TelegramColors.primaryDark, size: 28),
                const SizedBox(width: 10),
                Text(
                  'Telegram Drive',
                  style: TextStyle(
                    color: TelegramColors.textDark,
                    fontSize: 18,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 8),

          // Folder list
          Expanded(
            child: folders.isLoading
                ? const Center(
                    child: CircularProgressIndicator(strokeWidth: 2))
                : ListView(
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    children: [
                      // "Saved Messages" — default folder
                      _FolderItem(
                        icon: Icons.storage,
                        label: 'Saved Messages',
                        isActive: folders.activeFolderId == null,
                        onTap: () => _onFolderSelected(null),
                      ),
                      // API folders
                      for (final f in folders.folders)
                        _FolderItem(
                          icon: Icons.folder,
                          label: f.name,
                          isActive: f.id == folders.activeFolderId,
                          onTap: () => _onFolderSelected(f.id),
                        ),
                    ],
                  ),
          ),

          // Disconnect button
          const Divider(height: 1),
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 8, 8, 12),
            child: SizedBox(
              width: double.infinity,
              child: OutlinedButton.icon(
                onPressed: _onDisconnect,
                icon: const Icon(Icons.link_off, size: 18),
                label: const Text('Disconnect'),
                style: OutlinedButton.styleFrom(
                  foregroundColor: TelegramColors.subtextDark,
                  side: BorderSide(color: TelegramColors.borderDark),
                  padding: const EdgeInsets.symmetric(vertical: 12),
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(10),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  // ── Main content ────────────────────────────────────────────────

  Widget _buildMainContent(FilesProvider files, FoldersProvider folders) {
    return Column(
      children: [
        _TopBar(
          folderName: folders.activeFolderName,
          searchCtrl: _searchCtrl,
          onSearchChanged: _onSearchChanged,
          isGridView: _viewMode == _ViewMode.grid,
          onToggleView: () {
            setState(() {
              _viewMode = _viewMode == _ViewMode.grid
                  ? _ViewMode.list
                  : _ViewMode.grid;
            });
          },
          onMenuTap: () => _scaffoldKey.currentState?.openDrawer(),
          onDisconnect: _onDisconnect,
          showBackButton: folders.activeFolderId != null,
          onBackToRoot: () => _onFolderSelected(null),
        ),
        const Divider(height: 1),
        Expanded(
          child: RefreshIndicator(
            onRefresh: _onRefresh,
            child: _buildFileArea(files),
          ),
        ),
        _MiniPlayer(),
      ],
    );
  }

  // ── File area ────────────────────────────────────────────────────

  Widget _buildFileArea(FilesProvider files) {
    final folders = context.read<FoldersProvider>();

    // Show folder errors first — if we can't list folders, nothing works
    if (folders.error != null && folders.folders.isEmpty) {
      return _ErrorState(
        message: folders.error!,
        onRetry: () {
          folders.clearError();
          folders.loadFolders();
        },
      );
    }

    if (files.isLoading && files.displayedFiles.isEmpty) {
      return _ShimmerLoading(isGrid: _viewMode == _ViewMode.grid);
    }

    if (files.error != null && files.displayedFiles.isEmpty) {
      return _ErrorState(
        message: files.error!,
        onRetry: () {
          files.clearError();
          context.read<FoldersProvider>().loadFolders();
        },
      );
    }

    if (files.displayedFiles.isEmpty) {
      return _EmptyState(isSearching: files.searchQuery.isNotEmpty);
    }

    if (_viewMode == _ViewMode.grid) {
      return LayoutBuilder(
        builder: (context, constraints) {
          final crossAxisCount = constraints.maxWidth > 900
              ? 4
              : constraints.maxWidth > 600
                  ? 3
                  : 2;

          return GridView.builder(
            controller: _gridScrollCtrl,
            padding: const EdgeInsets.all(12),
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
              crossAxisCount: crossAxisCount,
              mainAxisExtent: 195,
              crossAxisSpacing: 10,
              mainAxisSpacing: 10,
            ),
            itemCount: files.displayedFiles.length,
            itemBuilder: (context, index) {
              final file = files.displayedFiles[index];
              return _FileCard(
                file: file,
                onTap: () => _onFileTap(file),
              );
            },
          );
        },
      );
    }

    // List view
    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: 4),
      itemCount: files.displayedFiles.length,
      itemBuilder: (context, index) {
        final file = files.displayedFiles[index];
        return _FileListItem(
          file: file,
          onTap: () => _onFileTap(file),
        );
      },
    );
  }
}

// =====================================================================
// Enums
// =====================================================================

enum _ViewMode { grid, list }

// =====================================================================
// _TopBar
// =====================================================================

class _TopBar extends StatelessWidget {
  const _TopBar({
    required this.folderName,
    required this.searchCtrl,
    required this.onSearchChanged,
    required this.isGridView,
    required this.onToggleView,
    required this.onMenuTap,
    required this.onDisconnect,
    required this.showBackButton,
    required this.onBackToRoot,
  });

  final String folderName;
  final TextEditingController searchCtrl;
  final ValueChanged<String> onSearchChanged;
  final bool isGridView;
  final VoidCallback onToggleView;
  final VoidCallback onMenuTap;
  final VoidCallback onDisconnect;
  final bool showBackButton;
  final VoidCallback onBackToRoot;

  @override
  Widget build(BuildContext context) {
    return Container(
      color: TelegramColors.surfaceDark,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      child: Row(
        children: [
          // Back arrow or hamburger menu
          IconButton(
            icon: Icon(showBackButton ? Icons.arrow_back : Icons.menu),
            onPressed: showBackButton ? onBackToRoot : onMenuTap,
            tooltip: showBackButton ? 'Back to root' : 'Open sidebar',
          ),

          // Breadcrumb
          Flexible(
            child: RichText(
              overflow: TextOverflow.ellipsis,
              text: TextSpan(
                style: TextStyle(color: TelegramColors.textDark, fontSize: 15),
                children: [
                  TextSpan(
                    text: 'Drive',
                    style: TextStyle(color: TelegramColors.subtextDark),
                    recognizer: TapGestureRecognizer()
                      ..onTap = onBackToRoot,
                  ),
                  TextSpan(
                    text: ' / ',
                    style: TextStyle(color: TelegramColors.subtextDark),
                  ),
                  TextSpan(
                    text: folderName,
                    style: TextStyle(
                      fontWeight: FontWeight.w600,
                      color: TelegramColors.textDark,
                    ),
                  ),
                ],
              ),
            ),
          ),

          const Spacer(),

          // File search
          SizedBox(
            width: 180,
            child: TextField(
              controller: searchCtrl,
              style: TextStyle(color: TelegramColors.textDark, fontSize: 14),
              decoration: InputDecoration(
                hintText: 'Search files…',
                hintStyle: TextStyle(color: TelegramColors.subtextDark),
                prefixIcon: Icon(Icons.search,
                    color: TelegramColors.subtextDark, size: 20),
                suffixIcon: searchCtrl.text.isNotEmpty
                    ? IconButton(
                        icon: Icon(Icons.clear,
                            color: TelegramColors.subtextDark, size: 18),
                        onPressed: () {
                          searchCtrl.clear();
                          onSearchChanged('');
                        },
                      )
                    : null,
                isDense: true,
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(10),
                  borderSide: BorderSide.none,
                ),
                filled: true,
                fillColor: TelegramColors.bgDark,
              ),
              onChanged: onSearchChanged,
            ),
          ),
          const SizedBox(width: 4),

          // View toggle
          IconButton(
            icon: Icon(isGridView ? Icons.view_list : Icons.grid_view),
            onPressed: onToggleView,
            tooltip: isGridView ? 'List view' : 'Grid view',
          ),

          // Disconnect
          IconButton(
            icon: const Icon(Icons.link_off),
            onPressed: onDisconnect,
            tooltip: 'Disconnect',
          ),
        ],
      ),
    );
  }
}

// =====================================================================
// _FolderItem
// =====================================================================

class _FolderItem extends StatelessWidget {
  const _FolderItem({
    required this.icon,
    required this.label,
    required this.isActive,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final bool isActive;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final color = isActive ? TelegramColors.primaryDark : TelegramColors.subtextDark;

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 1),
      child: Material(
        color: isActive
            ? TelegramColors.primaryDark.withValues(alpha: 0.12)
            : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
            child: Row(
              children: [
                Icon(icon, size: 20, color: color),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    label,
                    style: TextStyle(
                      color: isActive ? TelegramColors.primaryDark : TelegramColors.textDark,
                      fontWeight: isActive ? FontWeight.w600 : FontWeight.normal,
                      fontSize: 14,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                if (isActive)
                  Container(
                    width: 6,
                    height: 6,
                    decoration: const BoxDecoration(
                      color: TelegramColors.primaryDark,
                      shape: BoxShape.circle,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

// =====================================================================
// _FileCard (grid item)
// =====================================================================

class _FileCard extends StatefulWidget {
  const _FileCard({required this.file, required this.onTap});

  final TelegramFile file;
  final VoidCallback onTap;

  @override
  State<_FileCard> createState() => _FileCardState();
}

class _FileCardState extends State<_FileCard> {
  bool _pressed = false;

  @override
  Widget build(BuildContext context) {
    return AnimatedContainer(
      duration: const Duration(milliseconds: 150),
      transform: _pressed
          ? (Matrix4.diagonal3Values(0.96, 0.96, 1.0))
          : Matrix4.identity(),
      child: Card(
        margin: EdgeInsets.zero,
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: widget.onTap,
          onTapDown: (_) => setState(() => _pressed = true),
          onTapUp: (_) => setState(() => _pressed = false),
          onTapCancel: () => setState(() => _pressed = false),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // Thumbnail / icon area (4:3 ratio)
              AspectRatio(
                aspectRatio: 4 / 3,
                child: Container(
                  color: TelegramColors.bgDark,
                  child: Center(
                    child: Icon(
                      _fileTypeIcon(widget.file),
                      size: 40,
                      color: _fileTypeColor(widget.file),
                    ),
                  ),
                ),
              ),

              // File name & size
              Padding(
                padding: const EdgeInsets.fromLTRB(8, 8, 8, 6),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      widget.file.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: TelegramColors.textDark,
                        fontSize: 13,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    const SizedBox(height: 2),
                    Text(
                      widget.file.formatBytes(),
                      style: TextStyle(
                        color: TelegramColors.subtextDark,
                        fontSize: 11,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// =====================================================================
// _FileListItem (list view)
// =====================================================================

class _FileListItem extends StatelessWidget {
  const _FileListItem({required this.file, required this.onTap});

  final TelegramFile file;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Container(
        width: 44,
        height: 44,
        decoration: BoxDecoration(
          color: _fileTypeColor(file).withValues(alpha: 0.15),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Icon(
          _fileTypeIcon(file),
          color: _fileTypeColor(file),
          size: 22,
        ),
      ),
      title: Text(
        file.name,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          color: TelegramColors.textDark,
          fontSize: 14,
          fontWeight: FontWeight.w500,
        ),
      ),
      subtitle: Text(
        '${file.formatBytes()}  ·  ${_formatDate(file.createdAt)}',
        style: TextStyle(color: TelegramColors.subtextDark, fontSize: 12),
      ),
      trailing: file.isMedia
          ? Icon(Icons.play_circle_outlined,
              color: TelegramColors.primaryDark, size: 24)
          : null,
      onTap: onTap,
    );
  }
}

// =====================================================================
// _EmptyState
// =====================================================================

class _EmptyState extends StatelessWidget {
  const _EmptyState({this.isSearching = false});

  final bool isSearching;

  @override
  Widget build(BuildContext context) {
    return ListView(
      // ListView so RefreshIndicator works
      children: [
        SizedBox(
          height: MediaQuery.of(context).size.height * 0.5,
          child: Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  isSearching ? Icons.search_off : Icons.cloud_outlined,
                  size: 64,
                  color: TelegramColors.subtextDark,
                ),
                const SizedBox(height: 16),
                Text(
                  isSearching ? 'No results found' : 'No files yet',
                  style: TextStyle(
                    color: TelegramColors.textDark,
                    fontSize: 18,
                    fontWeight: FontWeight.w500,
                  ),
                ),
                const SizedBox(height: 8),
                Text(
                  isSearching
                      ? 'Try a different search term'
                      : 'Upload files from the desktop app',
                  style: TextStyle(color: TelegramColors.subtextDark, fontSize: 14),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

// =====================================================================
// _ErrorState
// =====================================================================

class _ErrorState extends StatelessWidget {
  const _ErrorState({required this.message, required this.onRetry});

  final String message;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.error_outline, size: 56, color: TelegramColors.subtextDark),
            const SizedBox(height: 16),
            Text(
              'Something went wrong',
              style: TextStyle(
                color: TelegramColors.textDark,
                fontSize: 18,
                fontWeight: FontWeight.w500,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              message,
              textAlign: TextAlign.center,
              style: TextStyle(color: TelegramColors.subtextDark, fontSize: 14),
            ),
            const SizedBox(height: 24),
            ElevatedButton.icon(
              onPressed: onRetry,
              icon: const Icon(Icons.refresh),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }
}

// =====================================================================
// _ShimmerLoading
// =====================================================================

class _ShimmerLoading extends StatefulWidget {
  const _ShimmerLoading({required this.isGrid});
  final bool isGrid;

  @override
  State<_ShimmerLoading> createState() => _ShimmerLoadingState();
}

class _ShimmerLoadingState extends State<_ShimmerLoading>
    with SingleTickerProviderStateMixin {
  late AnimationController _ctrl;
  late Animation<double> _anim;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1400),
    )..repeat(reverse: true);
    _anim = Tween<double>(begin: 0.3, end: 0.7).animate(_ctrl);
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _anim,
      builder: (ctx, _) => widget.isGrid ? _buildGrid() : _buildList(),
    );
  }

  Widget _buildGrid() {
    return GridView.builder(
      padding: const EdgeInsets.all(12),
      gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
        crossAxisCount: 2,
        mainAxisExtent: 195,
        crossAxisSpacing: 10,
        mainAxisSpacing: 10,
      ),
      itemCount: 8,
      itemBuilder: (ctx, i) => _ShimmerCard(opacity: _anim.value),
    );
  }

  Widget _buildList() {
    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: 4),
      itemCount: 10,
      itemBuilder: (ctx, i) => _ShimmerListItem(opacity: _anim.value),
    );
  }
}

class _ShimmerCard extends StatelessWidget {
  const _ShimmerCard({required this.opacity});
  final double opacity;

  @override
  Widget build(BuildContext context) {
    final shimmer = TelegramColors.surfaceDark.withValues(alpha: opacity);

    return Card(
      margin: EdgeInsets.zero,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          AspectRatio(
            aspectRatio: 4 / 3,
            child: Container(color: shimmer),
          ),
          Padding(
            padding: const EdgeInsets.all(8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  height: 10,
                  decoration: BoxDecoration(
                    color: shimmer,
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: 6),
                Container(
                  width: 60,
                  height: 8,
                  decoration: BoxDecoration(
                    color: shimmer,
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _ShimmerListItem extends StatelessWidget {
  const _ShimmerListItem({required this.opacity});
  final double opacity;

  @override
  Widget build(BuildContext context) {
    final shimmer = TelegramColors.surfaceDark.withValues(alpha: opacity);

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      child: Row(
        children: [
          Container(
            width: 44,
            height: 44,
            decoration: BoxDecoration(
              color: shimmer,
              borderRadius: BorderRadius.circular(10),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  height: 12,
                  width: double.infinity,
                  decoration: BoxDecoration(
                    color: shimmer,
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: 6),
                Container(
                  height: 10,
                  width: 120,
                  decoration: BoxDecoration(
                    color: shimmer,
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ═══════════════════════════════════════════════════════════════════════════════
// _MiniPlayer
// ═══════════════════════════════════════════════════════════════════════════════

/// A persistent mini-player bar shown at the bottom of the dashboard when
/// [MediaProvider.currentFile] is not null.
///
/// Displays the current file name, play/pause, and stop controls.
/// Tapping the bar (outside buttons) navigates to the full [MediaPlayerScreen].
class _MiniPlayer extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final media = context.watch<MediaProvider>();
    final file = media.currentFile;

    if (file == null) return const SizedBox.shrink();

    return GestureDetector(
      onTap: () {
        Navigator.pushNamed(
          context,
          '/media-player',
          arguments: {'file': file},
        );
      },
      child: Container(
        decoration: BoxDecoration(
          color: TelegramColors.surfaceDark,
          border: Border(
            top: BorderSide(color: TelegramColors.borderDark),
          ),
        ),
        padding: EdgeInsets.only(
          left: 16,
          right: 4,
          top: 8,
          bottom: 8 + MediaQuery.of(context).padding.bottom,
        ),
        child: Row(
          children: [
            // File icon
            Icon(
              file.isVideo ? Icons.play_circle_outline : Icons.music_note_outlined,
              color: TelegramColors.primaryDark,
              size: 20,
            ),
            const SizedBox(width: 12),

            // File name
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    file.name,
                    style: TextStyle(
                      color: TelegramColors.textDark,
                      fontSize: 14,
                      fontWeight: FontWeight.w500,
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                  const SizedBox(height: 2),
                  Text(
                    file.formatBytes(),
                    style: TextStyle(
                      color: TelegramColors.subtextDark,
                      fontSize: 11,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 4),

            // Play/pause button
            IconButton(
              icon: Icon(
                media.isPlaying ? Icons.pause_rounded : Icons.play_arrow_rounded,
                color: TelegramColors.textDark,
              ),
              onPressed: () => media.togglePlayPause(),
              tooltip: media.isPlaying ? 'Pause' : 'Play',
            ),

            // Stop button
            IconButton(
              icon: const Icon(Icons.stop_rounded, color: Colors.redAccent),
              onPressed: () {
                media.stop();
                // The widget will rebuild when currentFile becomes null,
                // which hides this mini-player.
              },
              tooltip: 'Stop',
            ),
          ],
        ),
      ),
    );
  }
}
