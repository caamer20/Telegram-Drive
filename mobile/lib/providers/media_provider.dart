import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:video_player/video_player.dart';

import '../models/api_models.dart';
import '../services/api_service.dart';

/// Defines the repeat behavior for media playback.
enum RepeatMode {
  /// No repeat — playback stops at the end of the playlist.
  none,

  /// Repeats the current file indefinitely.
  one,

  /// Repeats the entire playlist when reaching the end.
  all,
}

class MediaProvider extends ChangeNotifier {
  TelegramFile? _currentFile;
  List<TelegramFile> _playlist = [];
  int _currentIndex = 0;
  bool _isPlaying = false;
  bool _isShuffled = false;
  RepeatMode _repeatMode = RepeatMode.none;

  // Internal shuffled order
  List<int> _shuffledIndices = [];

  // Video player controller and playback state
  VideoPlayerController? _controller;
  bool _isInitialized = false;
  bool _hasError = false;
  String _errorMessage = '';
  Duration _position = Duration.zero;
  Duration _duration = Duration.zero;
  bool _isAdvancing = false;

  // Temp files to clean up on dispose
  final Set<String> _tempFiles = {};

  // API reference for auto-advance
  TelegramDriveApi? _api;

  // Download progress tracking (null = not downloading, 0.0-1.0 = progress)
  double? _downloadProgress;

  // Getters
  TelegramFile? get currentFile => _currentFile;
  List<TelegramFile> get playlist => List.unmodifiable(_playlist);
  int get currentIndex => _currentIndex;
  bool get isPlaying => _isPlaying;
  bool get isShuffled => _isShuffled;
  RepeatMode get repeatMode => _repeatMode;
  VideoPlayerController? get controller => _controller;
  bool get isInitialized => _isInitialized;
  bool get hasError => _hasError;
  String get errorMessage => _errorMessage;
  Duration get position => _position;
  Duration get duration => _duration;
  double? get downloadProgress => _downloadProgress;

  /// Stores the API reference for later use by auto-advance methods.
  void setApi(TelegramDriveApi api) {
    _api = api;
  }

  /// Starts playing the given [file], optionally within a [playlist] context.
  /// If a playlist is provided, it replaces the current playlist.
  /// If [api] is provided, the file is downloaded and initialised for
  /// playback via [_initFromApi]; otherwise state is updated but no
  /// controller is created (caller must provide api later).
  void playFile(TelegramFile file,
      {List<TelegramFile>? playlist, TelegramDriveApi? api}) {
    if (api != null) _api = api;

    if (playlist != null) {
      _playlist = List.from(playlist);
      _rebuildShuffledIndices();
    }

    _currentFile = file;
    _currentIndex = _playlist.indexOf(file);
    if (_currentIndex == -1) {
      _playlist.add(file);
      _currentIndex = _playlist.length - 1;
      _rebuildShuffledIndices();
    }
    _isInitialized = false; // Controller not ready yet
    _isPlaying = false;
    notifyListeners();

    final effectiveApi = api ?? _api;
    if (effectiveApi != null) {
      _initFromApi(effectiveApi, file);
    }
  }

  /// Advances to the next file in the playlist, respecting shuffle and
  /// repeat modes. Optionally accepts [api] for async initialisation.
  void playNext([TelegramDriveApi? api]) {
    if (_playlist.isEmpty) return;

    _isAdvancing = false;

    if (_isShuffled) {
      _advanceShuffled();
    } else {
      _advanceLinear();
    }

    _isInitialized = false;
    _isPlaying = false;
    notifyListeners();

    final effectiveApi = api ?? _api;
    if (effectiveApi != null && _currentFile != null) {
      _initFromApi(effectiveApi, _currentFile!);
    }
  }

  /// Goes back to the previous file in the playlist, respecting shuffle
  /// and repeat modes. Optionally accepts [api] for async initialisation.
  void playPrevious([TelegramDriveApi? api]) {
    if (_playlist.isEmpty) return;

    _isAdvancing = false;

    if (_isShuffled) {
      _retreatShuffled();
    } else {
      _retreatLinear();
    }

    _isInitialized = false;
    _isPlaying = false;
    notifyListeners();

    final effectiveApi = api ?? _api;
    if (effectiveApi != null && _currentFile != null) {
      _initFromApi(effectiveApi, _currentFile!);
    }
  }

  /// Toggles between playing and paused states.
  void togglePlayPause() {
    if (_controller == null || !_isInitialized) return;
    if (_isPlaying) {
      _controller!.pause();
      _isPlaying = false;
    } else {
      _controller!.play();
      _isPlaying = true;
    }
    notifyListeners();
  }

  /// Toggles shuffle mode on/off. Resets to linear order when disabled.
  void toggleShuffle() {
    _isShuffled = !_isShuffled;
    if (_isShuffled) {
      _rebuildShuffledIndices();
    }
    notifyListeners();
  }

  /// Cycles through repeat modes: none -> one -> all -> none.
  void toggleRepeatMode() {
    switch (_repeatMode) {
      case RepeatMode.none:
        _repeatMode = RepeatMode.one;
      case RepeatMode.one:
        _repeatMode = RepeatMode.all;
      case RepeatMode.all:
        _repeatMode = RepeatMode.none;
    }
    notifyListeners();
  }

  /// Adds a [file] to the end of the playlist.
  void addToPlaylist(TelegramFile file) {
    _playlist.add(file);
    _rebuildShuffledIndices();
    notifyListeners();
  }

  /// Removes the file at [index] from the playlist.
  void removeFromPlaylist(int index) {
    if (index < 0 || index >= _playlist.length) return;

    final wasCurrent = index == _currentIndex;
    _playlist.removeAt(index);

    if (wasCurrent) {
      if (_playlist.isEmpty) {
        _currentFile = null;
        _currentIndex = 0;
        _isPlaying = false;
      } else {
        _currentIndex = _currentIndex.clamp(0, _playlist.length - 1);
        _currentFile = _playlist[_currentIndex];
      }
    } else if (index < _currentIndex) {
      _currentIndex--;
    }

    _rebuildShuffledIndices();
    notifyListeners();
  }

  /// Clears the entire playlist and stops playback.
  void clearPlaylist() {
    _playlist = [];
    _shuffledIndices = [];
    _currentFile = null;
    _currentIndex = 0;
    _isPlaying = false;
    notifyListeners();
  }

  /// Stops playback, disposes the controller, and clears the current file.
  void stop() {
    _controller?.pause();
    _controller?.removeListener(_onControllerUpdate);
    _controller?.dispose();
    _controller = null;
    _isPlaying = false;
    _isInitialized = false;
    _hasError = false;
    _currentFile = null;
    _currentIndex = 0;
    _playlist = [];
    notifyListeners();
  }

  // ---- Internal helpers ----

  void _advanceLinear() {
    if (_currentIndex + 1 < _playlist.length) {
      _currentIndex++;
    } else {
      // Reached the end
      switch (_repeatMode) {
        case RepeatMode.all:
          _currentIndex = 0;
          break;
        case RepeatMode.one:
          // Stay on same file
          break;
        case RepeatMode.none:
          _isPlaying = false;
          return;
      }
    }
    _currentFile = _playlist[_currentIndex];
  }

  void _advanceShuffled() {
    final currentShufflePos = _shuffledIndices.indexOf(_currentIndex);
    if (currentShufflePos + 1 < _shuffledIndices.length) {
      _currentIndex = _shuffledIndices[currentShufflePos + 1];
    } else {
      switch (_repeatMode) {
        case RepeatMode.all:
          _currentIndex = _shuffledIndices.first;
          break;
        case RepeatMode.one:
          // Stay on same file
          break;
        case RepeatMode.none:
          _isPlaying = false;
          return;
      }
    }
    _currentFile = _playlist[_currentIndex];
  }

  void _retreatLinear() {
    if (_currentIndex > 0) {
      _currentIndex--;
    } else {
      switch (_repeatMode) {
        case RepeatMode.all:
          _currentIndex = _playlist.length - 1;
          break;
        case RepeatMode.one:
          // Stay on same file
          break;
        case RepeatMode.none:
          // Stay at beginning
          break;
      }
    }
    _currentFile = _playlist[_currentIndex];
  }

  void _retreatShuffled() {
    final currentShufflePos = _shuffledIndices.indexOf(_currentIndex);
    if (currentShufflePos > 0) {
      _currentIndex = _shuffledIndices[currentShufflePos - 1];
    } else {
      switch (_repeatMode) {
        case RepeatMode.all:
          _currentIndex = _shuffledIndices.last;
          break;
        case RepeatMode.one:
          // Stay on same file
          break;
        case RepeatMode.none:
          // Stay at first shuffled item
          break;
      }
    }
    _currentFile = _playlist[_currentIndex];
  }

  void _rebuildShuffledIndices() {
    if (_playlist.length <= 1) {
      _shuffledIndices = [for (int i = 0; i < _playlist.length; i++) i];
      return;
    }

    final indices = List.generate(_playlist.length, (i) => i);
    indices.shuffle(Random());
    _shuffledIndices = indices;
  }

  // ── Video player initialisation ────────────────────────────────────────

  /// Downloads the given [file] to a temp file and creates a
  /// [VideoPlayerController] for it. Falls back to streaming URL if
  /// download fails. Runs asynchronously and notifies listeners on
  /// completion or error.
  Future<void> _initFromApi(TelegramDriveApi api, TelegramFile file) async {
    // Tear down previous controller.
    _controller?.pause();
    _controller?.removeListener(_onControllerUpdate);
    _controller?.dispose();

    _isInitialized = false;
    _hasError = false;
    _errorMessage = '';
    _position = Duration.zero;
    _duration = Duration.zero;
    notifyListeners();

    VideoPlayerController newController;

    // ── Strategy 1: Download to temp file, then play from disk ──
    _downloadProgress = 0.0;
    notifyListeners();
    try {
      debugPrint('MediaProvider: strategy 1/3 (download to file)...');
      final tempDir = await getTemporaryDirectory();
      final safeName = file.name.replaceAll(RegExp(r'[^\w\.\-]'), '_');
      final tempFilePath = '${tempDir.path}/td_${file.id}_$safeName';
      final tempFile = File(tempFilePath);

      final success = await api.downloadToFile(
        file.id,
        tempFilePath,
        folderId: file.folderId,
      );

      if (!success) {
        throw const ApiException('Download returned false');
      }

      _tempFiles.add(tempFilePath);
      debugPrint('MediaProvider: strategy 1 succeeded (file: $tempFilePath)');
      newController = VideoPlayerController.file(tempFile);
    } catch (e) {
      debugPrint('MediaProvider: strategy 1 failed: $e');

      // ── Strategy 2: Stream via REST API download URL (port 8080) ──
      try {
        debugPrint('MediaProvider: strategy 2/3 (download URL stream)...');
        final downloadUrl = api.getDownloadStreamUrl(
          file.id,
          folderId: file.folderId,
        );
        debugPrint('MediaProvider: strategy 2 URL: $downloadUrl');
        newController = VideoPlayerController.networkUrl(
          Uri.parse(downloadUrl),
          httpHeaders: {'X-API-Key': api.apiKey},
        );
      } catch (e2) {
        debugPrint('MediaProvider: strategy 2 failed: $e2');

        // ── Strategy 3: Stream via streaming server (port 14201) ──
        debugPrint('MediaProvider: strategy 3/3 (stream server)...');
        final streamUrl = api.getStreamUrl(
          file.id,
          folderId: file.folderId,
        );
        newController = VideoPlayerController.networkUrl(
          Uri.parse(streamUrl),
          httpHeaders: {'X-API-Key': api.apiKey},
        );
      }
    }
    _downloadProgress = null;

    _controller = newController;
    newController.addListener(_onControllerUpdate);

    try {
      await newController.initialize();
      _isInitialized = true;
      await newController.play();
      _isPlaying = true;
      notifyListeners();
    } catch (e) {
      _hasError = true;
      _errorMessage = e.toString();
      _isPlaying = false;
      notifyListeners();
      debugPrint('MediaProvider: failed to load ${file.name}: $e');
    }
  }

  // ── Controller listener ────────────────────────────────────────────────

  void _onControllerUpdate() {
    if (_controller == null) return;
    final value = _controller!.value;

    _position = value.position;
    _duration = value.duration;

    // Auto-advance when playback completes naturally.
    if (!_isAdvancing &&
        value.isInitialized &&
        !value.isPlaying &&
        _isPlaying &&
        value.position >=
            value.duration - const Duration(milliseconds: 500) &&
        value.duration > const Duration(seconds: 1)) {
      _isAdvancing = true;
      playNext();
      return;
    }

    // Sync playing state from controller (handles edge cases).
    if (value.isInitialized) {
      final wasPlaying = _isPlaying;
      _isPlaying = value.isPlaying;
      if (wasPlaying != _isPlaying) {
        notifyListeners();
      }
    }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────

  @override
  void dispose() {
    _controller?.removeListener(_onControllerUpdate);
    _controller?.dispose();
    for (final path in _tempFiles) {
      File(path).delete().ignore();
    }
    super.dispose();
  }
}
