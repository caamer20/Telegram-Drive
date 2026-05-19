import 'package:flutter/foundation.dart';

import '../models/api_models.dart';
import '../services/api_service.dart';

class FilesProvider extends ChangeNotifier {
  final TelegramDriveApi _api;

  List<TelegramFile> _files = [];
  bool _isLoading = false;
  String? _error;
  String _searchQuery = '';
  List<TelegramFile> _searchResults = [];
  String? _searchError;

  FilesProvider(this._api);

  // Getters
  List<TelegramFile> get files => List.unmodifiable(_files);
  bool get isLoading => _isLoading;
  String? get error => _error;
  String get searchQuery => _searchQuery;
  List<TelegramFile> get searchResults => List.unmodifiable(_searchResults);
  String? get searchError => _searchError;

  /// Returns search results when a query is active, otherwise returns
  /// the full file list.
  List<TelegramFile> get displayedFiles {
    if (_searchQuery.isNotEmpty) {
      return _searchResults;
    }
    return _files;
  }

  /// Loads files for the given [folderId]. Pass null to load files
  /// from "Saved Messages".
  Future<void> loadFiles(int? folderId) async {
    _isLoading = true;
    _error = null;
    notifyListeners();

    try {
      _files = await _api.getFiles(folderId: folderId);
    } catch (e) {
      _error = e.toString();
      debugPrint('Failed to load files: $e');
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }

  /// Searches files using the API's search parameter with the given [query],
  /// optionally scoped to a specific [folderId].
  Future<void> search(String query, {int? folderId}) async {
    _searchQuery = query;
    _searchError = null;

    if (query.isEmpty) {
      _searchResults = [];
      notifyListeners();
      return;
    }

    _isLoading = true;
    notifyListeners();

    try {
      _searchResults = await _api.getFiles(search: query, folderId: folderId);
    } catch (e) {
      _searchError = e.toString();
      _searchResults = [];
      debugPrint('Search failed: $e');
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }

  /// Clears the current search query and results.
  void clearSearch() {
    _searchQuery = '';
    _searchResults = [];
    notifyListeners();
  }

  /// Clears the current error message.
  void clearError() {
    _error = null;
    notifyListeners();
  }
}
