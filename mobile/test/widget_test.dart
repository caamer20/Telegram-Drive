import 'package:flutter_test/flutter_test.dart';

import 'package:telegram_drive_mobile/models/api_models.dart';

void main() {
  test('TelegramFile model creates from JSON', () {
    final json = {
      'id': 123,
      'name': 'test.mp4',
      'size': 1048576,
      'mime_type': 'video/mp4',
      'file_ext': 'mp4',
      'created_at': '2024-01-15T10:30:00Z',
      'folder_id': 456,
    };

    final file = TelegramFile.fromJson(json);

    expect(file.id, 123);
    expect(file.name, 'test.mp4');
    expect(file.size, 1048576);
    expect(file.mimeType, 'video/mp4');
    expect(file.fileExt, 'mp4');
    expect(file.isVideo, isTrue);
    expect(file.isAudio, isFalse);
    expect(file.isMedia, isTrue);
    expect(file.isImage, isFalse);
    expect(file.formatBytes(), '1.00 MB');
  });

  test('TelegramFile handles nullable fields', () {
    final json = {
      'id': 456,
      'name': 'document.pdf',
      'size': 500000,
      'created_at': '2024-03-20T14:00:00Z',
    };

    final file = TelegramFile.fromJson(json);

    expect(file.id, 456);
    expect(file.name, 'document.pdf');
    expect(file.mimeType, isNull);
    expect(file.fileExt, isNull);
    expect(file.folderId, isNull);
    expect(file.isPdf, isFalse); // null extension
    expect(file.isMedia, isFalse);
  });

  test('TelegramFolder model round-trip', () {
    final folder = TelegramFolder(id: 1, name: 'My Channel');

    final json = folder.toJson();
    final restored = TelegramFolder.fromJson(json);

    expect(restored.id, 1);
    expect(restored.name, 'My Channel');
    expect(restored, equals(folder));
  });

  test('TelegramFile formatBytes handles edge cases', () {
    expect(TelegramFile.formatFileSize(0), '0 Bytes');
    expect(TelegramFile.formatFileSize(1023), '1023.00 Bytes');
    expect(TelegramFile.formatFileSize(1024), '1.00 KB');
    expect(TelegramFile.formatFileSize(1048576), '1.00 MB');
    expect(TelegramFile.formatFileSize(1073741824), '1.00 GB');
  });
}
