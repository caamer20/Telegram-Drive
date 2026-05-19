import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../services/api_service.dart';

class ConnectionProvider extends ChangeNotifier {
  String? _baseUrl;
  String? _apiKey;
  bool _isConnected = false;
  bool _isLoading = false;
  String? _error;

  TelegramDriveApi? _api;

  // Getters
  String? get baseUrl => _baseUrl;
  String? get apiKey => _apiKey;
  bool get isConnected => _isConnected;
  bool get isLoading => _isLoading;
  String? get error => _error;
  TelegramDriveApi? get api => _api;

  /// Attempts to connect to the Telegram Drive server at [baseUrl]
  /// using [apiKey] for authentication.
  Future<void> connect(String baseUrl, String apiKey) async {
    _isLoading = true;
    _error = null;
    notifyListeners();

    try {
      final api = TelegramDriveApi(baseUrl: baseUrl, apiKey: apiKey);
      final healthy = await api.checkHealth();

      if (!healthy) {
        throw Exception('Server unreachable — check the URL and ensure the API server is running');
      }

      // Validate the API key by calling an authenticated endpoint.
      final keyValid = await api.validateAuth();
      if (!keyValid) {
        throw Exception('Invalid API key — generate a new one in Settings → REST API on the desktop app');
      }

      // Fetch stream token for media playback.
      await api.fetchStreamInfo();

      _baseUrl = baseUrl;
      _apiKey = apiKey;
      _api = api;
      _isConnected = true;
      _error = null;

      // Persist connection settings
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString('base_url', baseUrl);
      await prefs.setString('api_key', apiKey);
    } catch (e) {
      _isConnected = false;
      _api = null;
      _error = e.toString();
    } finally {
      _isLoading = false;
      notifyListeners();
    }
  }

  /// Loads a previously saved connection from SharedPreferences
  /// and automatically reconnects.
  Future<void> loadSavedConnection() async {
    final prefs = await SharedPreferences.getInstance();
    final savedBaseUrl = prefs.getString('base_url');
    final savedApiKey = prefs.getString('api_key');

    if (savedBaseUrl != null && savedApiKey != null) {
      await connect(savedBaseUrl, savedApiKey);
    }
  }

  /// Disconnects from the server, clears saved credentials,
  /// and resets all state.
  Future<void> disconnect() async {
    _baseUrl = null;
    _apiKey = null;
    _api = null;
    _isConnected = false;
    _isLoading = false;
    _error = null;

    final prefs = await SharedPreferences.getInstance();
    await prefs.remove('base_url');
    await prefs.remove('api_key');

    notifyListeners();
  }

  /// Clears the current error message.
  void clearError() {
    _error = null;
    notifyListeners();
  }
}
