/// Full-screen media player for video and audio playback.
///
/// Uses [MediaProvider] for playlist state and [TelegramDriveApi] for
/// stream URLs. Manages its own [VideoPlayerController] lifecycle.
/// Supports video (full player) and audio (animated gradient disc) modes,
/// playlist browsing, shuffle, repeat, and landscape orientation.
library;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:video_player/video_player.dart';

import '../models/api_models.dart';
import '../providers/media_provider.dart' as media_provider;
import '../services/api_service.dart';
import '../theme/app_theme.dart' show TelegramColors;

// ── Constants ──────────────────────────────────────────────────────────────

/// Primary accent colour shared across the player.
const Color _kAmber = TelegramColors.primaryDark;

/// Background (pure black for immersive playback).
const Color _kBg = Colors.black;

/// Surface colour for controls and playlist.
const Color _kSurface = Color(0xFF1A1A2E);

/// Secondary / subtle text.
const Color _kSubtext = Color(0xFF8E9BAA);

/// Tablet breakpoint at which the playlist becomes a persistent sidebar
/// instead of a bottom-sheet overlay.
const double _kTabletBreakpoint = 600.0;

// ── Screen ─────────────────────────────────────────────────────────────────

/// Full-screen media player that manages playback of a playlist of media
/// files (video or audio).
///
/// Reads [MediaProvider] for playlist state and uses [TelegramDriveApi] to
/// build stream URLs.
class MediaPlayerScreen extends StatefulWidget {
  final media_provider.MediaProvider mediaProvider;
  final TelegramDriveApi api;

  const MediaPlayerScreen({
    super.key,
    required this.mediaProvider,
    required this.api,
  });

  @override
  State<MediaPlayerScreen> createState() => _MediaPlayerScreenState();
}

class _MediaPlayerScreenState extends State<MediaPlayerScreen>
    with SingleTickerProviderStateMixin {
  // ── Audio disc rotation ───────────────────────────────────────────────────

  late final AnimationController _spinController;

  // ── Playlist panel state ──────────────────────────────────────────────────

  bool _showPlaylistPanel = false;

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  @override
  void initState() {
    super.initState();
    _spinController = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 4),
    );
    widget.mediaProvider.addListener(_onProviderChanged);

    // Allow landscape rotation for video playback.
    SystemChrome.setPreferredOrientations([
      DeviceOrientation.portraitUp,
      DeviceOrientation.landscapeLeft,
      DeviceOrientation.landscapeRight,
    ]);
  }

  @override
  void dispose() {
    widget.mediaProvider.removeListener(_onProviderChanged);
    _spinController.dispose();

    // Restore portrait lock.
    SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
    super.dispose();
  }

  // ── Provider listener ─────────────────────────────────────────────────────

  void _onProviderChanged() {
    if (!mounted) return;
    _syncSpin();
    setState(() {});
  }

  /// Syncs the disc spin animation with the provider's playing state.
  void _syncSpin() {
    final p = widget.mediaProvider;
    if (p.isPlaying && p.currentFile?.isAudio == true) {
      _spinController.repeat();
    } else {
      _spinController.stop();
    }
  }

  // ── Controls ────────────────────────────────────────────────────────────────

  void _playIndex(int index) {
    final playlist = widget.mediaProvider.playlist;
    if (index < 0 || index >= playlist.length) return;
    widget.mediaProvider.playFile(playlist[index], api: widget.api);
  }

  void _playNext() => widget.mediaProvider.playNext(widget.api);

  void _playPrevious() => widget.mediaProvider.playPrevious(widget.api);

  void _togglePlayPause() {
    widget.mediaProvider.togglePlayPause();
    _syncSpin();
  }

  void _toggleShuffle() => widget.mediaProvider.toggleShuffle();

  void _cycleRepeatMode() => widget.mediaProvider.toggleRepeatMode();

  void _onSeek(double fraction) {
    final c = widget.mediaProvider.controller;
    final dur = widget.mediaProvider.duration;
    if (c == null || !widget.mediaProvider.isInitialized) return;
    final seekTo = Duration(
      milliseconds: (fraction * dur.inMilliseconds)
          .round()
          .clamp(0, dur.inMilliseconds),
    );
    c.seekTo(seekTo);
  }

  // ── Derived state ──────────────────────────────────────────────────────────

  TelegramFile? get _currentFile => widget.mediaProvider.currentFile;

  bool get _isVideo => _currentFile?.isVideo ?? false;

  String get _remainingTime {
    final remaining = widget.mediaProvider.duration - widget.mediaProvider.position;
    if (remaining.isNegative || remaining == Duration.zero) return '0:00';
    return '-${_formatDuration(remaining)}';
  }

  int get _playlistLength => widget.mediaProvider.playlist.length;

  // ── Formatting ─────────────────────────────────────────────────────────────

  static String _formatDuration(Duration d) {
    final hours = d.inHours;
    final minutes = d.inMinutes.remainder(60);
    final seconds = d.inSeconds.remainder(60);
    if (hours > 0) {
      return '${hours.toString().padLeft(2, '0')}:${minutes.toString().padLeft(2, '0')}:${seconds.toString().padLeft(2, '0')}';
    }
    return '${minutes.toString().padLeft(2, '0')}:${seconds.toString().padLeft(2, '0')}';
  }

  // ── Build ──────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: !_showPlaylistPanel || _playlistLength <= 0,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && _showPlaylistPanel) {
          setState(() => _showPlaylistPanel = false);
        }
      },
      child: Scaffold(
        backgroundColor: _kBg,
        body: SafeArea(
          child: LayoutBuilder(
            builder: (context, constraints) {
              final isWide = constraints.maxWidth > _kTabletBreakpoint;

              // Tablet: persistent sidebar alongside the player.
              if (isWide && _showPlaylistPanel) {
                return Row(
                  children: [
                    Expanded(child: _buildMainColumn()),
                    _buildPlaylistSidebar(),
                  ],
                );
              }

              // Phone: Stack with optional bottom sheet.
              return Stack(
                children: [
                  _buildMainColumn(),
                  if (!isWide && _showPlaylistPanel)
                    Positioned(
                      left: 0,
                      right: 0,
                      bottom: 0,
                      child: _buildPlaylistSheet(),
                    ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }

  /// The main content column (top bar + player + controls + progress).
  Widget _buildMainColumn() {
    return Column(
      children: [
        _buildTopBar(),
        Expanded(child: _buildPlayerArea()),
        if (!_isVideo) _buildProgressBarCentered(),
        _buildControls(),
        if (_isVideo) _buildProgressBar(),
        _buildBottomInfo(),
        SizedBox(height: MediaQuery.of(context).padding.bottom + 8),
      ],
    );
  }

  // ── Top bar ───────────────────────────────────────────────────────────────

  Widget _buildTopBar() {
    final index = widget.mediaProvider.currentIndex;
    final fileName = _currentFile?.name ?? '';

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
      child: Row(
        children: [
          // Close / back
          IconButton(
            icon: const Icon(Icons.keyboard_arrow_down, color: Colors.white),
            onPressed: () => Navigator.of(context).pop(),
            tooltip: 'Close',
          ),

          // File name + index
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  fileName,
                  style: const TextStyle(
                    color: Colors.white,
                    fontSize: 15,
                    fontWeight: FontWeight.w500,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                if (_playlistLength > 1)
                  Text(
                    '${index + 1} of $_playlistLength',
                    style: const TextStyle(
                      color: _kSubtext,
                      fontSize: 12,
                    ),
                  ),
              ],
            ),
          ),

          // Playlist toggle
          if (_playlistLength > 1)
            IconButton(
              icon: Icon(
                Icons.playlist_play,
                color: _showPlaylistPanel ? _kAmber : Colors.white70,
              ),
              onPressed: () =>
                  setState(() => _showPlaylistPanel = !_showPlaylistPanel),
              tooltip: 'Playlist',
            ),
        ],
      ),
    );
  }

  // ── Player area ───────────────────────────────────────────────────────────

  Widget _buildPlayerArea() {
    if (_currentFile == null) return _buildEmptyState();

    if (widget.mediaProvider.hasError) return _buildErrorState();

    if (!widget.mediaProvider.isInitialized) {
      return _buildLoadingState(
        downloadProgress: widget.mediaProvider.downloadProgress,
      );
    }

    if (_isVideo) return _buildVideoPlayer();

    return _buildAudioPlayer();
  }

  Widget _buildVideoPlayer() {
    if (widget.mediaProvider.controller == null ||
        !widget.mediaProvider.isInitialized) {
      return _buildLoadingState(
        downloadProgress: widget.mediaProvider.downloadProgress,
      );
    }

    final isPlaying = widget.mediaProvider.isPlaying;

    final controller = widget.mediaProvider.controller!;
    final aspectRatio = controller.value.aspectRatio;

    return Stack(
      alignment: Alignment.center,
      children: [
        // Video content with proper aspect ratio
        Center(
          child: aspectRatio > 0
              ? AspectRatio(
                  aspectRatio: aspectRatio,
                  child: VideoPlayer(controller),
                )
              : VideoPlayer(controller),
        ),

        // Tap to play/pause (covers whole video area)
        GestureDetector(
          onTap: _togglePlayPause,
          child: Container(color: Colors.transparent),
        ),

        // Previous overlay
        Positioned(
          left: 8,
          child: IconButton(
            icon: const Icon(Icons.skip_previous,
                color: Colors.white70, size: 32),
            onPressed: widget.mediaProvider.currentIndex > 0
                ? _playPrevious
                : null,
            tooltip: 'Previous',
          ),
        ),

        // Next overlay
        Positioned(
          right: 8,
          child: IconButton(
            icon: const Icon(Icons.skip_next, color: Colors.white70, size: 32),
            onPressed: widget.mediaProvider.currentIndex < _playlistLength - 1
                ? _playNext
                : null,
            tooltip: 'Next',
          ),
        ),

        // Centred play icon fade overlay
        if (!isPlaying)
          Container(
            decoration: const BoxDecoration(
              shape: BoxShape.circle,
              color: Color(0x80000000),
            ),
            padding: const EdgeInsets.all(20),
            child: const Icon(Icons.play_arrow, color: Colors.white, size: 48),
          ),
      ],
    );
  }

  Widget _buildAudioPlayer() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        const Spacer(flex: 2),

        // Spinning disc
        _buildAudioDisc(),

        const SizedBox(height: 24),

        // File name
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 32),
          child: Text(
            _currentFile?.name ?? '',
            style: const TextStyle(
              color: Colors.white,
              fontSize: 20,
              fontWeight: FontWeight.w600,
            ),
            textAlign: TextAlign.center,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
          ),
        ),

        // File size
        if (_currentFile != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              _currentFile!.formatBytes(),
              style: const TextStyle(color: _kSubtext, fontSize: 13),
            ),
          ),

        const Spacer(flex: 2),
      ],
    );
  }

  Widget _buildAudioDisc() {
    final isPlaying = widget.mediaProvider.isPlaying;

    return GestureDetector(
      onTap: _togglePlayPause,
      child: AnimatedBuilder(
        animation: _spinController,
        builder: (context, child) {
          return Transform.rotate(
            angle: _spinController.value * 2 * 3.141592653589793,
            child: child,
          );
        },
        child: Container(
          width: 220,
          height: 220,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            gradient: const SweepGradient(
              startAngle: 0,
              endAngle: 3.14159 * 2,
              colors: [
                Color(0xFF3A3A3A),
                Color(0xFF1A1A1A),
                Color(0xFF2A2A2A),
                Color(0xFF111111),
                Color(0xFF3A3A3A),
              ],
            ),
            border: Border.all(color: Colors.white24, width: 2),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: 0.5),
                blurRadius: 24,
                spreadRadius: 4,
              ),
            ],
          ),
          child: Center(
            child: Container(
              width: 70,
              height: 70,
              decoration: const BoxDecoration(
                shape: BoxShape.circle,
                color: Colors.black,
              ),
              child: Icon(
                isPlaying
                    ? Icons.music_note_rounded
                    : Icons.play_arrow_rounded,
                color: _kAmber.withValues(alpha: 0.7),
                size: 36,
              ),
            ),
          ),
        ),
      ),
    );
  }

  // ── Controls row ──────────────────────────────────────────────────────────

  Widget _buildControls() {
    final index = widget.mediaProvider.currentIndex;
    final hasPrevious = index > 0;
    final hasNext = index < _playlistLength - 1;
    final isPlaying = widget.mediaProvider.isPlaying;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          // Shuffle
          IconButton(
            icon: Icon(
              Icons.shuffle,
              color: widget.mediaProvider.isShuffled
                  ? _kAmber
                  : Colors.white54,
              size: 24,
            ),
            onPressed: _toggleShuffle,
            tooltip: widget.mediaProvider.isShuffled
                ? 'Shuffle: on'
                : 'Shuffle: off',
          ),

          const SizedBox(width: 12),

          // Previous
          IconButton(
            icon: Icon(
              Icons.skip_previous,
              color: hasPrevious ? Colors.white : Colors.white24,
              size: 32,
            ),
            onPressed: hasPrevious ? _playPrevious : null,
            tooltip: 'Previous',
          ),

          const SizedBox(width: 12),

          // Play / Pause (large amber circle)
          Container(
            decoration: const BoxDecoration(
              shape: BoxShape.circle,
              color: _kAmber,
            ),
            child: IconButton(
              icon: Icon(
                isPlaying ? Icons.pause : Icons.play_arrow,
                color: Colors.black,
                size: 32,
              ),
              onPressed: _togglePlayPause,
              tooltip: isPlaying ? 'Pause' : 'Play',
              splashRadius: 28,
            ),
          ),

          const SizedBox(width: 12),

          // Next
          IconButton(
            icon: Icon(
              Icons.skip_next,
              color: hasNext ? Colors.white : Colors.white24,
              size: 32,
            ),
            onPressed: hasNext ? _playNext : null,
            tooltip: 'Next',
          ),

          const SizedBox(width: 12),

          // Repeat
          IconButton(
            icon: _repeatIcon(),
            color: widget.mediaProvider.repeatMode != media_provider.RepeatMode.none
                ? _kAmber
                : Colors.white54,
            onPressed: _cycleRepeatMode,
            tooltip: _repeatTooltip(),
          ),
        ],
      ),
    );
  }

  Icon _repeatIcon() {
    final icon = switch (widget.mediaProvider.repeatMode) {
      media_provider.RepeatMode.none => Icons.repeat,
      media_provider.RepeatMode.one => Icons.repeat_one_on,
      media_provider.RepeatMode.all => Icons.repeat_on,
    };
    return Icon(icon);
  }

  String _repeatTooltip() {
    return switch (widget.mediaProvider.repeatMode) {
      media_provider.RepeatMode.none => 'Repeat: off',
      media_provider.RepeatMode.one => 'Repeat: one',
      media_provider.RepeatMode.all => 'Repeat: all',
    };
  }

  // ── Progress bar ──────────────────────────────────────────────────────────

  /// Progress slider with time labels, positioned at the bottom for video.
  Widget _buildProgressBar() {
    if (widget.mediaProvider.duration == Duration.zero) {
      return const SizedBox(height: 36);
    }

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Row(
        children: [
          SizedBox(
            width: 46,
            child: Text(_formatDuration(widget.mediaProvider.position),
                style: const TextStyle(color: _kSubtext, fontSize: 12)),
          ),
          Expanded(child: _buildProgressSlider()),
          SizedBox(
            width: 46,
            child: Text(_remainingTime,
                style: const TextStyle(color: _kSubtext, fontSize: 12),
                textAlign: TextAlign.end),
          ),
        ],
      ),
    );
  }

  /// Centred progress bar for audio (between disc and controls).
  Widget _buildProgressBarCentered() {
    if (widget.mediaProvider.duration == Duration.zero) {
      return const SizedBox.shrink();
    }

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 8),
      child: Column(
        children: [
          _buildProgressSlider(),
          const SizedBox(height: 4),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(_formatDuration(widget.mediaProvider.position),
                  style: const TextStyle(color: _kSubtext, fontSize: 11)),
              Text(_remainingTime,
                  style: const TextStyle(color: _kSubtext, fontSize: 11)),
            ],
          ),
        ],
      ),
    );
  }

  /// Shared slider widget wrapped in a [ValueListenableBuilder] for
  /// efficient position updates from the video player controller.
  Widget _buildProgressSlider() {
    final controller = widget.mediaProvider.controller;
    if (controller == null || !widget.mediaProvider.isInitialized) {
      return const SizedBox.shrink();
    }
    return ValueListenableBuilder<VideoPlayerValue>(
      valueListenable: controller,
      builder: (context, value, child) {
        final duration = value.duration;
        final position = value.position;
        final fraction = duration.inMilliseconds > 0
            ? (position.inMilliseconds / duration.inMilliseconds)
                .clamp(0.0, 1.0)
            : 0.0;

        return SliderTheme(
          data: SliderThemeData(
            activeTrackColor: _kAmber,
            inactiveTrackColor: Colors.white24,
            thumbColor: _kAmber,
            overlayColor: _kAmber.withValues(alpha: 0.12),
            trackHeight: 3,
            thumbShape:
                const RoundSliderThumbShape(enabledThumbRadius: 6),
          ),
          child: Slider(
            value: fraction,
            onChanged: (v) {
              _onSeek(v);
            },
          ),
        );
      },
    );
  }

  // ── Bottom info ──────────────────────────────────────────────────────────

  Widget _buildBottomInfo() {
    if (_playlistLength <= 1) return const SizedBox.shrink();

    final index = widget.mediaProvider.currentIndex;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Text(
            '${index + 1} of $_playlistLength',
            style: const TextStyle(color: _kSubtext, fontSize: 13),
          ),
          const SizedBox(width: 12),
          Text(
            widget.mediaProvider.isShuffled ? 'Shuffled' : 'In order',
            style: TextStyle(
              color: widget.mediaProvider.isShuffled
                  ? _kAmber
                  : _kSubtext,
              fontSize: 13,
            ),
          ),
        ],
      ),
    );
  }

  // ── Playlist panel ────────────────────────────────────────────────────────

  /// Phone: bottom-sheet style panel shown over the player.
  Widget _buildPlaylistSheet() {
    final playlist = widget.mediaProvider.playlist;
    final currentIndex = widget.mediaProvider.currentIndex;

    return Container(
      height: MediaQuery.of(context).size.height * 0.55,
      decoration: const BoxDecoration(
        color: _kSurface,
        borderRadius: BorderRadius.vertical(top: Radius.circular(16)),
      ),
      child: Column(
        children: [
          // Drag handle
          Container(
            margin: const EdgeInsets.symmetric(vertical: 8),
            width: 36,
            height: 4,
            decoration: BoxDecoration(
              color: Colors.white24,
              borderRadius: BorderRadius.circular(2),
            ),
          ),

          // Header
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
            child: Row(
              children: [
                const Text(
                  'Playlist',
                  style: TextStyle(
                    color: Colors.white,
                    fontSize: 16,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const Spacer(),
                Text(
                  '${playlist.length} files',
                  style: const TextStyle(color: _kSubtext, fontSize: 13),
                ),
              ],
            ),
          ),

          const Divider(color: Colors.white12, height: 1),

          // List
          Expanded(
            child: ListView.builder(
              padding: const EdgeInsets.symmetric(vertical: 4),
              itemCount: playlist.length,
              itemBuilder: (context, index) {
                final file = playlist[index];
                final isCurrent = index == currentIndex;
                return _PlaylistItemTile(
                  file: file,
                  index: index,
                  isCurrent: isCurrent,
                  onTap: isCurrent
                      ? null
                      : () {
                          setState(() => _showPlaylistPanel = false);
                          _playIndex(index);
                        },
                );
              },
            ),
          ),
        ],
      ),
    );
  }

  /// Tablet: persistent sidebar shown alongside the player.
  Widget _buildPlaylistSidebar() {
    final playlist = widget.mediaProvider.playlist;
    final currentIndex = widget.mediaProvider.currentIndex;

    return Container(
      width: 260,
      decoration: BoxDecoration(
        color: _kSurface,
        border: const Border(
          left: BorderSide(color: Colors.white12, width: 0.5),
        ),
      ),
      child: Column(
        children: [
          // Header
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 20, 16, 8),
            child: Row(
              children: [
                const Icon(Icons.playlist_play,
                    color: Colors.white70, size: 20),
                const SizedBox(width: 8),
                const Text(
                  'Playlist',
                  style: TextStyle(
                    color: Colors.white,
                    fontSize: 15,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const Spacer(),
                Text(
                  '${playlist.length}',
                  style: const TextStyle(color: _kSubtext, fontSize: 13),
                ),
              ],
            ),
          ),

          const Divider(color: Colors.white12, height: 1),

          // List
          Expanded(
            child: ListView.builder(
              padding: const EdgeInsets.symmetric(vertical: 4),
              itemCount: playlist.length,
              itemBuilder: (context, index) {
                final file = playlist[index];
                final isCurrent = index == currentIndex;
                return _PlaylistItemTile(
                  file: file,
                  index: index,
                  isCurrent: isCurrent,
                  onTap: isCurrent ? null : () => _playIndex(index),
                );
              },
            ),
          ),
        ],
      ),
    );
  }

  // ── Empty / Error / Loading ──────────────────────────────────────────────

  Widget _buildEmptyState() {
    return const Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.music_note_outlined, color: Colors.white24, size: 64),
          SizedBox(height: 16),
          Text(
            'No media selected',
            style: TextStyle(color: Colors.white54, fontSize: 16),
          ),
        ],
      ),
    );
  }

  Widget _buildErrorState() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.error_outline, color: Colors.redAccent, size: 56),
            const SizedBox(height: 16),
            const Text(
              'Failed to load media',
              style: TextStyle(color: Colors.white, fontSize: 18),
            ),
            const SizedBox(height: 8),
            Text(
              widget.mediaProvider.errorMessage,
              style: const TextStyle(color: _kSubtext, fontSize: 13),
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 24),
            FilledButton.icon(
              onPressed: () => widget.mediaProvider.playFile(
                widget.mediaProvider.currentFile!,
                api: widget.api,
              ),
              icon: const Icon(Icons.refresh),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildLoadingState({double? downloadProgress}) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const SizedBox(
            width: 48,
            height: 48,
            child: CircularProgressIndicator(strokeWidth: 3),
          ),
          const SizedBox(height: 16),
          if (downloadProgress != null) ...[
            const Text(
              'Downloading\u2026',
              style: TextStyle(color: Colors.white70, fontSize: 15),
            ),
            const SizedBox(height: 12),
            const Padding(
              padding: EdgeInsets.symmetric(horizontal: 48),
              child: LinearProgressIndicator(),
            ),
            const SizedBox(height: 8),
            Text(
              _currentFile?.formatBytes() ?? '',
              style: const TextStyle(color: _kSubtext, fontSize: 12),
            ),
          ] else
            const Text(
              'Buffering\u2026',
              style: TextStyle(color: Colors.white70, fontSize: 15),
            ),
        ],
      ),
    );
  }
}

// ── Private widgets ──────────────────────────────────────────────────────────

/// A single tile in the playlist list.
class _PlaylistItemTile extends StatelessWidget {
  final TelegramFile file;
  final int index;
  final bool isCurrent;
  final VoidCallback? onTap;

  const _PlaylistItemTile({
    required this.file,
    required this.index,
    required this.isCurrent,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Material(
      color: isCurrent ? _kAmber.withValues(alpha: 0.12) : Colors.transparent,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          child: Row(
            children: [
              // Index or play icon
              SizedBox(
                width: 28,
                child: isCurrent
                    ? const Icon(Icons.play_arrow_rounded,
                        color: _kAmber, size: 18)
                    : Text(
                        '${index + 1}',
                        style: const TextStyle(
                          color: _kSubtext,
                          fontSize: 13,
                        ),
                      ),
              ),

              const SizedBox(width: 8),

              // File type icon
              Icon(
                file.isVideo ? Icons.movie_outlined : Icons.music_note_outlined,
                color: isCurrent ? _kAmber : _kSubtext,
                size: 20,
              ),

              const SizedBox(width: 10),

              // Name
              Expanded(
                child: Text(
                  file.name,
                  style: TextStyle(
                    color: isCurrent ? Colors.white : Colors.white70,
                    fontSize: 14,
                    fontWeight: isCurrent ? FontWeight.w600 : FontWeight.normal,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
