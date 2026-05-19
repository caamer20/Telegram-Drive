/// Full-screen PDF viewer with pinch-to-zoom, page navigation, and swipe.
///
/// Downloads the PDF to a temp directory on init using
/// [TelegramDriveApi.downloadToFile], displays it with [PDFView], and cleans
/// up the temp file on dispose.
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_pdfview/flutter_pdfview.dart';
import 'package:path_provider/path_provider.dart';

import '../models/api_models.dart';
import '../services/api_service.dart';

// ── Constants ──────────────────────────────────────────────────────────────

/// Background colour (dark theme matching the app).
const Color _kBg = Color(0xFF0E1621);

/// Secondary / subtle text colour.
const Color _kSubtext = Color(0xFF8E9BAA);

// ── Load state ─────────────────────────────────────────────────────────────

/// Internal load states for the viewer.
enum _LoadState { downloading, loaded, error }

// ── Screen ─────────────────────────────────────────────────────────────────

/// Full-screen PDF viewer for reading documents.
///
/// Downloads the PDF file to a temp directory, renders it with
/// [PDFView] (flutter_pdfview), and cleans up on dispose.
class PdfViewerScreen extends StatefulWidget {
  /// The file to view (must be a PDF).
  final TelegramFile file;

  /// API client used to download the file.
  final TelegramDriveApi api;

  const PdfViewerScreen({
    super.key,
    required this.file,
    required this.api,
  });

  @override
  State<PdfViewerScreen> createState() => _PdfViewerScreenState();
}

class _PdfViewerScreenState extends State<PdfViewerScreen> {
  _LoadState _state = _LoadState.downloading;
  String? _errorMessage;
  String? _tempFilePath;
  int _totalPages = 0;
  int _currentPage = 0;

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  @override
  void initState() {
    super.initState();
    // Allow all orientations for landscape reading.
    SystemChrome.setPreferredOrientations([
      DeviceOrientation.portraitUp,
      DeviceOrientation.landscapeLeft,
      DeviceOrientation.landscapeRight,
    ]);
    _downloadFile();
  }

  @override
  void dispose() {
    // Restore portrait lock.
    SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
    _cleanupTempFile();
    super.dispose();
  }

  // ── File management ───────────────────────────────────────────────────────

  /// Deletes the downloaded temp file if it exists.
  void _cleanupTempFile() {
    if (_tempFilePath != null) {
      final file = File(_tempFilePath!);
      if (file.existsSync()) {
        file.deleteSync();
      }
    }
  }

  /// Downloads the PDF to a temp directory.
  ///
  /// Updates [_state] to [__LoadState.loaded] on success or
  /// [__LoadState.error] on failure.
  Future<void> _downloadFile() async {
    try {
      final tempDir = await getTemporaryDirectory();
      final safeName =
          widget.file.name.replaceAll(RegExp(r'[^\w\.\-]'), '_');
      final tempPath = '${tempDir.path}/pdf_${widget.file.id}_$safeName';

      final success = await widget.api.downloadToFile(
        widget.file.id,
        tempPath,
        folderId: widget.file.folderId,
      );

      if (!mounted) return;

      if (success) {
        setState(() {
          _tempFilePath = tempPath;
          _state = _LoadState.loaded;
        });
      } else {
        setState(() {
          _state = _LoadState.error;
          _errorMessage = 'Download failed';
        });
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _state = _LoadState.error;
        _errorMessage = e.toString();
      });
    }
  }

  // ── Build ─────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: _kBg,
      appBar: _buildAppBar(),
      body: _buildBody(),
    );
  }

  AppBar _buildAppBar() {
    return AppBar(
      backgroundColor: _kBg,
      iconTheme: const IconThemeData(color: Colors.white),
      title: Text(
        widget.file.name,
        style: const TextStyle(
          color: Colors.white,
          fontSize: 16,
          fontWeight: FontWeight.w500,
        ),
        overflow: TextOverflow.ellipsis,
      ),
      // Page indicator in the actions area when loaded.
      actions: _state == _LoadState.loaded && _totalPages > 0
          ? [
              Padding(
                padding: const EdgeInsets.only(right: 16),
                child: Center(
                  child: Text(
                    '${_currentPage + 1} / $_totalPages',
                    style: const TextStyle(
                      color: _kSubtext,
                      fontSize: 14,
                    ),
                  ),
                ),
              ),
            ]
          : null,
    );
  }

  Widget _buildBody() {
    switch (_state) {
      case _LoadState.downloading:
        return _buildLoadingState();
      case _LoadState.error:
        return _buildErrorState();
      case _LoadState.loaded:
        if (_tempFilePath == null) return _buildErrorState();
        return _buildPdfViewer();
    }
  }

  Widget _buildLoadingState() {
    return const Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            width: 48,
            height: 48,
            child: CircularProgressIndicator(strokeWidth: 3),
          ),
          SizedBox(height: 16),
          Text(
            'Downloading PDF\u2026',
            style: TextStyle(color: Colors.white70, fontSize: 15),
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
            const Icon(Icons.error_outline,
                color: Colors.redAccent, size: 56),
            const SizedBox(height: 16),
            const Text(
              'Failed to load PDF',
              style: TextStyle(color: Colors.white, fontSize: 18),
            ),
            if (_errorMessage != null) ...[
              const SizedBox(height: 8),
              Text(
                _errorMessage!,
                style: const TextStyle(color: _kSubtext, fontSize: 13),
                textAlign: TextAlign.center,
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
              ),
            ],
            const SizedBox(height: 24),
            FilledButton.icon(
              onPressed: () {
                setState(() {
                  _state = _LoadState.downloading;
                  _errorMessage = null;
                });
                _downloadFile();
              },
              icon: const Icon(Icons.refresh),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildPdfViewer() {
    return PDFView(
      filePath: _tempFilePath!,
      enableSwipe: true,
      swipeHorizontal: false,
      autoSpacing: true,
      pageFling: true,
      onRender: (pages) {
        if (!mounted) return;
        setState(() {
          _totalPages = pages ?? 0;
        });
      },
      onError: (error) {
        if (!mounted) return;
        setState(() {
          _state = _LoadState.error;
          _errorMessage = 'PDF render error: $error';
        });
      },
      onPageError: (page, error) {
        debugPrint('PDF page error (page $page): $error');
      },
      onPageChanged: (page, total) {
        if (!mounted) return;
        setState(() {
          _currentPage = page ?? 0;
          _totalPages = total ?? 0;
        });
      },
    );
  }
}
