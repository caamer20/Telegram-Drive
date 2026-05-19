/// REST API client for the Telegram Drive server.
///
/// Provides typed methods for all available API endpoints, handling
/// authentication, request serialization, and error mapping.
library;

import 'dart:convert';
import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';

import '../models/api_models.dart';

// ── Exception types ──────────────────────────────────────────────────────

/// Thrown when an API request fails.
class ApiException implements Exception {
  /// Human-readable description of what went wrong.
  final String message;

  /// HTTP status code that caused the failure, if applicable.
  final int? statusCode;

  /// Raw response body, if available.
  final String? body;

  const ApiException(this.message, {this.statusCode, this.body});

  @override
  String toString() {
    final sb = StringBuffer('ApiException: $message');
    if (statusCode != null) sb.write(' (status: $statusCode)');
    return sb.toString();
  }
}

/// Thrown when the server is unreachable.
class ApiConnectionException extends ApiException {
  const ApiConnectionException(super.message, {super.statusCode, super.body});
}

// ── API Client ───────────────────────────────────────────────────────────

/// Client for the Telegram Drive REST API.
///
/// Usage:
/// ```dart
/// final api = TelegramDriveApi(
///   baseUrl: 'http://192.168.1.100:8080',
///   apiKey: 'your-api-key',
/// );
/// final folders = await api.getFolders();
/// ```
class TelegramDriveApi {
  /// Base URL of the REST API server (e.g., `http://192.168.1.100:8080`).
  final String baseUrl;

  /// API key for X-API-Key authentication.
  final String apiKey;

  /// The streaming server port (defaults to 14201).
  final int streamPort;

  /// Stream authentication token, fetched from the health endpoint.
  String? _streamToken;

  /// Underlying HTTP client. Can be overridden for testing.
  final http.Client _client;

  /// Creates an [http.Client] that bypasses the system proxy.
  ///
  /// This is necessary because the host may have `http_proxy` or
  /// `https_proxy` environment variables set (e.g. for a VPN or
  /// local proxy tool), which would prevent the mobile app from
  /// reaching the Telegram Drive API server running on the same machine.
  static http.Client _createDirectClient() {
    final inner = HttpClient()
      ..findProxy = (uri) => 'DIRECT';
    return IOClient(inner);
  }

  /// Creates a new [TelegramDriveApi] client.
  ///
  /// [baseUrl] should include protocol and port, e.g. `http://192.168.1.100:8080`.
  /// [apiKey] is the secret key set in the desktop app's API configuration.
  /// [streamPort] defaults to 14201 (the streaming server port).
  TelegramDriveApi({
    required this.baseUrl,
    required this.apiKey,
    this.streamPort = 14201,
    http.Client? client,
  }) : _client = client ?? _createDirectClient();

  /// Disposes the underlying HTTP client.
  void dispose() {
    _client.close();
  }

  /// Extracts the host from the configured [baseUrl].
  ///
  /// Used to construct the streaming server URL on a different port.
  String get _host => Uri.parse(baseUrl).host;

  // ── Common helpers ────────────────────────────────────────────────────

  Map<String, String> get _headers => {
        'Content-Type': 'application/json',
        'X-API-Key': apiKey,
      };

  /// Wraps a GET request with a 15-second timeout, decoding the JSON response.
  Future<dynamic> _get(String path, {Map<String, String>? queryParams, Duration timeout = const Duration(seconds: 15)}) async {
    final uri = Uri.parse('$baseUrl$path').replace(queryParameters: queryParams);
    try {
      final response = await _client.get(uri, headers: _headers).timeout(timeout);
      return _handleResponse(response);
    } on TimeoutException {
      throw ApiConnectionException('Request timed out after ${timeout.inSeconds}s');
    } on http.ClientException catch (e) {
      throw ApiConnectionException(
        'Connection failed: ${e.message}',
      );
    } on FormatException catch (e) {
      throw ApiException('Invalid response format: ${e.message}');
    }
  }

  /// Validates the HTTP response and decodes JSON body.
  dynamic _handleResponse(http.Response response) {
    if (response.statusCode >= 200 && response.statusCode < 300) {
      if (response.body.isEmpty) return null;
      return jsonDecode(response.body);
    }

    String message;
    switch (response.statusCode) {
      case 401:
        message = 'Unauthorized – check your API key';
        break;
      case 403:
        message = 'Forbidden';
        break;
      case 404:
        message = 'Resource not found';
        break;
      case 429:
        message = 'Rate limited – try again later';
        break;
      case >= 500:
        message = 'Server error';
        break;
      default:
        message = 'Request failed';
    }

    throw ApiException(message, statusCode: response.statusCode, body: response.body);
  }

  // ── Endpoints ─────────────────────────────────────────────────────────

  /// Checks whether the Telegram Drive API server is reachable.
  ///
  /// Returns `true` if the server responds with `{"status": "ok"}`.
  Future<bool> checkHealth() async {
    try {
      final data = await _get('/api/v1/health', timeout: const Duration(seconds: 5));
      if (data is Map && data['status'] == 'ok') return true;
      return false;
    } on ApiException {
      return false;
    }
  }

  /// Validates that the API key is accepted by hitting an authenticated
  /// endpoint. Returns `true` if the server accepted the key (HTTP 200).
  ///
  /// Call this after [checkHealth] to ensure the credentials are valid
  /// before proceeding with data loading.
  Future<bool> validateAuth() async {
    try {
      await _get('/api/v1/folders', timeout: const Duration(seconds: 15));
      return true;
    } on ApiException catch (e) {
      if (e.statusCode == 401) return false;
      // Non-auth errors (server busy, etc.) — treat as OK, data will
      // surface the error when actually loaded.
      return true;
    }
  }

  /// Fetches stream authentication info from the health endpoint.
  ///
  /// Returns `true` if a valid stream token was obtained.
  Future<bool> fetchStreamInfo() async {
    try {
      final data = await _get('/api/v1/health');
      if (data is Map) {
        final token = data['stream_token'] as String?;
        if (token != null) {
          _streamToken = token;
          return true;
        }
      }
      return false;
    } on ApiException {
      return false;
    }
  }

  /// Retrieves all folders accessible in Telegram Drive.
  ///
  /// Returns an empty list on failure rather than throwing.
  Future<List<TelegramFolder>> getFolders() async {
    try {
      final data = await _get('/api/v1/folders');
      if (data is! List) return [];
      return data
          .map((e) => TelegramFolder.fromJson(e as Map<String, dynamic>))
          .toList();
    } on ApiException {
      rethrow;
    } catch (e) {
      debugPrint('getFolders: unexpected error: $e');
      return [];
    }
  }

  /// Retrieves files with optional filtering.
  ///
  /// Parameters:
  /// - [folderId]: Filter to a specific folder.
  /// - [page]: Page number (1-indexed).
  /// - [limit]: Items per page (default 50).
  /// - [search]: Optional search query to filter file names.
  ///
  /// Returns an empty list on failure rather than throwing.
  Future<List<TelegramFile>> getFiles({
    int? folderId,
    int page = 1,
    int limit = 50,
    String? search,
  }) async {
    final params = <String, String>{
      'page': page.toString(),
      'limit': limit.toString(),
    };
    if (folderId != null) params['folder_id'] = folderId.toString();
    if (search != null && search.isNotEmpty) params['search'] = search;

    try {
      final data = await _get('/api/v1/files', queryParams: params);
      if (data is! Map) return [];
      final filesList = data['files'];
      if (filesList is! List) return [];
      return filesList
          .map((e) => TelegramFile.fromJson(e as Map<String, dynamic>))
          .toList();
    } on ApiException {
      rethrow;
    } catch (e) {
      debugPrint('getFiles: unexpected error: $e');
      return [];
    }
  }

  /// Fetches a single file by its Telegram message ID.
  ///
  /// Returns `null` if the file is not found (HTTP 404).
  Future<TelegramFile?> getFile(int messageId, {int? folderId}) async {
    final params = <String, String>{};
    if (folderId != null) params['folder_id'] = folderId.toString();

    try {
      final data = await _get('/api/v1/files/$messageId', queryParams: params.isNotEmpty ? params : null);
      if (data is! Map<String, dynamic>) return null;
      return TelegramFile.fromJson(data);
    } on ApiException catch (e) {
      if (e.statusCode == 404) return null;
      rethrow;
    } catch (e) {
      debugPrint('getFile: unexpected error: $e');
      return null;
    }
  }

  /// Builds the stream URL for playing a media file.
  ///
  /// The streaming server runs on [streamPort] (default 14201)
  /// and serves media over HTTP for video/audio playback.
  ///
  /// If [streamToken] is provided, it is included as a query parameter.
  String getStreamUrl(
    int messageId, {
    int? folderId,
    String? streamToken,
  }) {
    final folderPath = folderId?.toString() ?? 'me';
    final token = streamToken ?? _streamToken;
    final uri = Uri(
      scheme: 'http',
      host: _host,
      port: streamPort,
      path: '/stream/$folderPath/$messageId',
      queryParameters: token != null ? {'token': token} : null,
    );
    return uri.toString();
  }

  /// Builds the download URL for a file.
  ///
  /// Returns a URL that triggers a file download from the REST API server.
  String getDownloadUrl(int messageId, {int? folderId}) {
    final params = <String, String>{};
    if (folderId != null) params['folder_id'] = folderId.toString();

    final uri = Uri.parse('$baseUrl/api/v1/files/$messageId/download')
        .replace(queryParameters: params.isNotEmpty ? params : null);
    return uri.toString();
  }

  /// Builds a download URL with the API key as a query parameter.
  ///
  /// Used when the caller cannot set custom HTTP headers (e.g.
  /// [VideoPlayerController.networkUrl]). Appends `api_key` as a query
  /// parameter to authenticate the request.
  String getDownloadStreamUrl(int messageId, {int? folderId}) {
    final params = <String, String>{
      'api_key': apiKey,
    };
    if (folderId != null) params['folder_id'] = folderId.toString();

    final uri = Uri.parse('$baseUrl/api/v1/files/$messageId/download')
        .replace(queryParameters: params);
    return uri.toString();
  }

  /// Downloads a file's raw bytes using the proxy-bypassing HTTP client.
  ///
  /// Uses [getDownloadUrl] to construct the URL, then fetches via [_client]
  /// which bypasses any system proxy. Returns the raw bytes for local storage.
  Future<Uint8List> downloadFileBytes(int messageId, {int? folderId}) async {
    final url = getDownloadUrl(messageId, folderId: folderId);
    try {
      final response = await _client
          .get(Uri.parse(url), headers: _headers)
          .timeout(const Duration(seconds: 30));
      if (response.statusCode == 200) {
        return response.bodyBytes;
      }
      throw ApiException('Download failed', statusCode: response.statusCode);
    } on ApiException {
      rethrow;
    } on TimeoutException {
      throw ApiConnectionException('Download timed out');
    } catch (e) {
      throw ApiConnectionException('Download connection failed: $e');
    }
  }

  /// Downloads a file by streaming to [filePath] using the proxy-bypassing client.
  /// Returns `true` on success, `false` on failure.
  Future<bool> downloadToFile(int messageId, String filePath, {int? folderId}) async {
    final url = getDownloadUrl(messageId, folderId: folderId);
    try {
      final request = http.Request('GET', Uri.parse(url));
      request.headers.addAll(_headers);
      final response = await _client.send(request);
      if (response.statusCode == 200) {
        final file = File(filePath);
        await response.stream.pipe(file.openWrite());
        return true;
      }
      debugPrint('downloadToFile: HTTP ${response.statusCode}');
      return false;
    } catch (e) {
      debugPrint('downloadToFile failed: $e');
      return false;
    }
  }
}
