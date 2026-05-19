/// Data models matching the Telegram Drive REST API responses.
///
/// These models map directly to the JSON payloads returned by the
/// Telegram Drive desktop application's REST API server.
library;

/// Represents a folder (a Telegram channel/chat) in Telegram Drive.
///
/// Folders are used to organize files, mirroring Telegram channels
/// where each channel acts as a virtual folder.
class TelegramFolder {
  /// Unique identifier for the folder (Telegram chat ID).
  final int id;

  /// Display name of the folder (channel title).
  final String name;

  /// Creates a [TelegramFolder] with the given properties.
  const TelegramFolder({
    required this.id,
    required this.name,
  });

  /// Constructs a [TelegramFolder] from a JSON map.
  factory TelegramFolder.fromJson(Map<String, dynamic> json) {
    return TelegramFolder(
      id: json['id'] as int,
      name: json['name'] as String,
    );
  }

  /// Converts this folder to a JSON-compatible map.
  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
    };
  }

  @override
  String toString() => 'TelegramFolder(id: $id, name: $name)';

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is TelegramFolder && runtimeType == other.runtimeType && id == other.id;

  @override
  int get hashCode => id.hashCode;
}

/// Represents a file stored in Telegram Drive.
///
/// Files are stored as Telegram messages with attached media or documents.
/// The [fileExt] and [mimeType] fields allow classification into
/// video, audio, image, or other categories.
class TelegramFile {
  /// Unique identifier for the file (Telegram message ID).
  final int id;

  /// File name including extension (e.g., "vacation.mp4").
  final String name;

  /// File size in bytes.
  final int size;

  /// MIME type of the file, if available (e.g., "video/mp4").
  final String? mimeType;

  /// File extension without the dot (e.g., "mp4", "jpg").
  final String? fileExt;

  /// ISO 8601 timestamp of when the file was created/uploaded.
  final String createdAt;

  /// ID of the parent folder, or `null` if the file is in the root.
  final int? folderId;

  /// Creates a [TelegramFile] with the given properties.
  const TelegramFile({
    required this.id,
    required this.name,
    required this.size,
    this.mimeType,
    this.fileExt,
    required this.createdAt,
    this.folderId,
  });

  /// Constructs a [TelegramFile] from a JSON map.
  ///
  /// All fields are expected to be present in the JSON. Nullable fields
  /// gracefully handle missing or `null` values.
  factory TelegramFile.fromJson(Map<String, dynamic> json) {
    final name = json['name'] as String;
    final fileExt = json['file_ext'] as String?;
    return TelegramFile(
      id: json['id'] as int,
      name: name,
      size: json['size'] as int,
      mimeType: json['mime_type'] as String?,
      fileExt: fileExt ?? _extractExtension(name),
      createdAt: json['created_at'] as String,
      folderId: json['folder_id'] as int?,
    );
  }

  /// Extracts the file extension from the file name.
  ///
  /// Returns the substring after the last `.` in [name], lowercased,
  /// or `null` if no extension is found.
  static String? _extractExtension(String name) {
    final dot = name.lastIndexOf('.');
    if (dot == -1 || dot == name.length - 1) return null;
    return name.substring(dot + 1).toLowerCase();
  }

  /// Converts this file to a JSON-compatible map.
  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
      'size': size,
      'mime_type': mimeType,
      'file_ext': fileExt,
      'created_at': createdAt,
      'folder_id': folderId,
    };
  }

  // ── File type classification ────────────────────────────────────────────

  static const List<String> _videoExtensions = [
    'mp4', 'webm', 'ogg', 'mov', 'mkv', 'avi',
  ];

  static const List<String> _audioExtensions = [
    'mp3', 'wav', 'aac', 'flac', 'm4a', 'opus',
  ];

  static const List<String> _imageExtensions = [
    'jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif',
  ];

  /// Returns `true` if this file has a video extension.
  bool get isVideo {
    final ext = fileExt?.toLowerCase();
    return ext != null && _videoExtensions.contains(ext);
  }

  /// Returns `true` if this file has an audio extension.
  bool get isAudio {
    final ext = fileExt?.toLowerCase();
    return ext != null && _audioExtensions.contains(ext);
  }

  /// Returns `true` if this file has an image extension.
  bool get isImage {
    final ext = fileExt?.toLowerCase();
    return ext != null && _imageExtensions.contains(ext);
  }

  /// Returns `true` if this file is a video based on file extension.
  ///
  /// Equivalent to [isVideo]; provided for naming consistency with the
  /// web client's helper functions.
  bool isVideoFile() => isVideo;

  /// Returns `true` if this file is an audio based on file extension.
  ///
  /// Equivalent to [isAudio]; provided for naming consistency with the
  /// web client's helper functions.
  bool isAudioFile() => isAudio;

  /// Returns `true` if this file is an image based on file extension.
  ///
  /// Equivalent to [isImage]; provided for naming consistency with the
  /// web client's helper functions.
  bool isImageFile() => isImage;

  /// Returns `true` if this file is a PDF document.
  bool get isPdf {
    final ext = fileExt?.toLowerCase();
    return ext == 'pdf';
  }

  /// Returns `true` if this file is a playable media file (video or audio).
  bool get isMedia => isVideo || isAudio;

  /// Formats the file [size] as a human-readable string.
  ///
  /// Examples: "1.5 MB", "342 KB", "1.2 GB"
  String formatBytes() {
    return formatFileSize(size);
  }

  /// Static helper to format byte counts into human-readable strings.
  ///
  /// Examples: `formatFileSize(1048576)` → "1.0 MB"
  static String formatFileSize(int bytes, {int decimals = 2}) {
    if (bytes <= 0) return '0 Bytes';
    const suffixes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const k = 1024;
    final i = (bytes.bitLength / (k.bitLength - 1)).floor().clamp(0, suffixes.length - 1);
    final value = bytes / (1 << (i * 10));
    return '${value.toStringAsFixed(decimals)} ${suffixes[i]}';
  }

  @override
  String toString() => 'TelegramFile(id: $id, name: $name, size: $size)';

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is TelegramFile && runtimeType == other.runtimeType && id == other.id;

  @override
  int get hashCode => id.hashCode;
}

/// Represents a single item in a media playback playlist.
///
/// Used by the media player to track which file is currently playing
/// and navigate between consecutive files.
class PlaylistItem {
  /// The file to play.
  final TelegramFile file;

  /// Index position in the playlist.
  final int index;

  /// Creates a [PlaylistItem] with the given [file] and [index].
  const PlaylistItem({required this.file, required this.index});

  @override
  String toString() => 'PlaylistItem(index: $index, file: ${file.name})';
}
