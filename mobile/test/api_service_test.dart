import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:telegram_drive_mobile/models/api_models.dart';
import 'package:telegram_drive_mobile/services/api_service.dart';

/// Helper that builds a [MockClient] returning [statusCode] and [body].
http.Client _mockClient(int statusCode, dynamic body,
    {Map<String, String>? Function(http.Request)? inspect}) {
  return MockClient((request) async {
    if (inspect != null) inspect(request);
    return http.Response(
      body is String ? body : jsonEncode(body),
      statusCode,
      headers: {'content-type': 'application/json'},
    );
  });
}

/// Standard mock health-check response.
Map<String, dynamic> _healthBody({String? token}) => {
      'status': 'ok',
      'version': '1.4.2',
      'stream_token': token ?? 'abc123',
      'stream_port': 14201,
    };

/// A folder fixture returned by the mock server.
final _folderFixture = [
  {'id': 1, 'name': 'My Videos'},
  {'id': 2, 'name': 'Documents'},
];

/// A FilesResponse fixture.
final _filesFixture = {
  'files': [
    {
      'id': 101,
      'folder_id': 1,
      'name': 'vacation.mp4',
      'size': 10_485_760,
      'mime_type': 'video/mp4',
      'created_at': '2026-01-15 10:00:00',
    },
    {
      'id': 102,
      'folder_id': null,
      'name': 'readme.txt',
      'size': 1024,
      'mime_type': 'text/plain',
      'created_at': '2026-01-16 12:30:00',
    },
  ],
  'page': 1,
  'limit': 50,
  'total': 2,
};

void main() {
  group('TelegramDriveApi', () {
    const baseUrl = 'http://192.168.1.100:8550';
    const apiKey = 'test-api-key';

    // ── checkHealth ───────────────────────────────────────────────

    test('checkHealth returns true when server responds ok', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      expect(await api.checkHealth(), isTrue);
    });

    test('checkHealth returns false on non-ok status', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, {'status': 'error'}),
      );
      expect(await api.checkHealth(), isFalse);
    });

    test('checkHealth returns false on error response', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(500, 'server error'),
      );
      expect(await api.checkHealth(), isFalse);
    });

    // ── validateAuth ──────────────────────────────────────────────

    test('validateAuth returns true on 200 from folders endpoint', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _folderFixture),
      );
      expect(await api.validateAuth(), isTrue);
    });

    test('validateAuth returns false on 401', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(401, {
          'error': {'code': 'UNAUTHORIZED', 'message': 'Invalid API key'},
        }),
      );
      expect(await api.validateAuth(), isFalse);
    });

    test('validateAuth returns true on non-auth error', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(503, 'service unavailable'),
      );
      expect(await api.validateAuth(), isTrue);
    });

    // ── fetchStreamInfo ───────────────────────────────────────────

    test('fetchStreamInfo returns true and stores token', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody(token: 'stream-token-99')),
      );
      expect(await api.fetchStreamInfo(), isTrue);
      // The token is stored internally; getStreamUrl should use it.
      final url = api.getStreamUrl(42);
      expect(url, contains('token=stream-token-99'));
    });

    test('fetchStreamInfo returns false when token is missing', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, {
          'status': 'ok',
          'version': '1.4.2',
          'stream_port': 14201,
        }),
      );
      expect(await api.fetchStreamInfo(), isFalse);
    });

    // ── getFolders ────────────────────────────────────────────────

    test('getFolders returns list of TelegramFolder', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _folderFixture),
      );
      final folders = await api.getFolders();
      expect(folders, hasLength(2));
      expect(folders[0].id, 1);
      expect(folders[0].name, 'My Videos');
      expect(folders[1].id, 2);
      expect(folders[1].name, 'Documents');
    });

    test('getFolders returns empty list for non-array response', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, {'not': 'an array'}),
      );
      expect(await api.getFolders(), isEmpty);
    });

    test('getFolders throws ApiException on HTTP error', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(500, 'server error'),
      );
      expect(
        () => api.getFolders(),
        throwsA(isA<ApiException>()),
      );
    });

    // ── getFiles ──────────────────────────────────────────────────

    test('getFiles returns list of TelegramFile', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _filesFixture),
      );
      final files = await api.getFiles();
      expect(files, hasLength(2));
      expect(files[0].id, 101);
      expect(files[0].name, 'vacation.mp4');
      expect(files[0].mimeType, 'video/mp4');
      expect(files[1].name, 'readme.txt');
    });

    test('getFiles sends correct pagination params for folder', () async {
      Uri? capturedUri;
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _filesFixture, inspect: (req) {
          capturedUri = req.url;
          return null;
        }),
      );
      await api.getFiles(folderId: 1, page: 2, limit: 10);
      expect(capturedUri!.path, '/api/v1/files');
      expect(capturedUri!.queryParameters['folder_id'], '1');
      expect(capturedUri!.queryParameters['page'], '2');
      expect(capturedUri!.queryParameters['limit'], '10');
    });

    test('getFiles sends search param when provided', () async {
      Uri? capturedUri;
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _filesFixture, inspect: (req) {
          capturedUri = req.url;
          return null;
        }),
      );
      await api.getFiles(search: 'vacation');
      expect(capturedUri!.queryParameters['search'], 'vacation');
    });

    test('getFiles returns empty list for invalid response', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, {'files': 'not a list'}),
      );
      expect(await api.getFiles(), isEmpty);
    });

    // ── getFile ───────────────────────────────────────────────────

    test('getFile returns TelegramFile when found', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, (_filesFixture['files'] as List<dynamic>)[0]),
      );
      final file = await api.getFile(101);
      expect(file, isNotNull);
      expect(file!.name, 'vacation.mp4');
    });

    test('getFile returns null on 404', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(404, {'error': 'not found'}),
      );
      expect(await api.getFile(999), isNull);
    });

    test('getFile throws on non-404 error', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(500, 'server error'),
      );
      expect(
        () => api.getFile(101),
        throwsA(isA<ApiException>()),
      );
    });

    // ── getStreamUrl ──────────────────────────────────────────────

    test('getStreamUrl builds correct URL with token', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      await api.fetchStreamInfo();
      final url = api.getStreamUrl(42, folderId: 1);
      expect(url, startsWith('http://192.168.1.100:14201/stream/1/42'));
      expect(url, contains('token='));
    });

    test('getStreamUrl builds URL with explicit token', () {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      final url = api.getStreamUrl(42, streamToken: 'explicit-token');
      expect(url, contains('token=explicit-token'));
    });

    test('getStreamUrl defaults to "me" when no folderId', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      await api.fetchStreamInfo();
      final url = api.getStreamUrl(42);
      expect(url, contains('/stream/me/42'));
    });

    // ── getDownloadUrl ────────────────────────────────────────────

    test('getDownloadUrl builds correct URL without folder', () {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      final url = api.getDownloadUrl(42);
      expect(url,
          'http://192.168.1.100:8550/api/v1/files/42/download');
    });

    test('getDownloadUrl includes folder_id param', () {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: _mockClient(200, _healthBody()),
      );
      final url = api.getDownloadUrl(42, folderId: 1);
      expect(url, contains('folder_id=1'));
    });

    // ── Auth header ───────────────────────────────────────────────

    test('sends X-API-Key header on all requests', () async {
      Map<String, String>? capturedHeaders;
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: 'my-secret-key',
        client: _mockClient(200, _healthBody(),
            inspect: (req) => capturedHeaders = req.headers),
      );
      await api.checkHealth();
      expect(capturedHeaders!['x-api-key'], 'my-secret-key');
      expect(capturedHeaders!['content-type'], 'application/json');
    });

    // ── Error handling ────────────────────────────────────────────

    test('throws ApiConnectionException on client error', () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: MockClient((_) async {
          throw http.ClientException('Connection refused');
        }),
      );
      expect(
        () => api.getFolders(),
        throwsA(isA<ApiConnectionException>()),
      );
    });

    test('throws ApiConnectionException with correct message on client error',
        () async {
      final api = TelegramDriveApi(
        baseUrl: baseUrl,
        apiKey: apiKey,
        client: MockClient((_) async {
          throw http.ClientException('Connection refused');
        }),
      );
      try {
        await api.getFolders();
        fail('Expected exception');
      } on ApiConnectionException catch (e) {
        expect(e.message, contains('Connection refused'));
      }
    });
  });
}
