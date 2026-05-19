import 'package:flutter/foundation.dart';

import '../models/api_models.dart';
import '../services/api_service.dart';

class FoldersProvider extends ChangeNotifier {
  final TelegramDriveApi _api;

  List<TelegramFolder> _folders = [];
  bool _isLoading = false;
  String? _error;
  int? _activeFolderId;

  FoldersProvider(this._api);

  // Getters
  List<TelegramFolder> get folders => List.unmodifiable(_folders);
  bool get isLoading => _isLoading;
  String? get error => _error;
  int? get activeFolderId => _activeFolderId;

  /// Returns the display name of the active folder.
  /// Returns "Saved Messages" when no folder is selected (null id).
  String get activeFolderName {
    if (_activeFolderId == null) return 'Saved Messages';
    final folder = _folders.where((f) => f.id == _activeFolderId);
    return folder.isNotEmpty ? folder.first.name : 'Unknown';
  }

  /// Loads the list of folders from the API.
  Future<void> loadFolders() async {
    _isLoading = true;
    _error = null;
    notifyListeners();

    try {
      _folders = await _api.getFolders();
    } catch (e) {
      _error = e.toString();
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }

  /// Clears the current error.
  void clearError() {
    _error = null;
    notifyListeners();
  }

  /// Sets the active folder by [folderId]. Pass null to select
  /// "Saved Messages".
  void setActiveFolder(int? folderId) {
    _activeFolderId = folderId;
    notifyListeners();
  }
}
