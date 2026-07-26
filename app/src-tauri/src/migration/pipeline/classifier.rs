#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Video,
    Image,
    Other,
}

/// Phân loại file dựa trên extension.
/// Nếu extension không khớp, fallback về Other.
pub fn classify_file(filename: &str) -> FileCategory {
    let lower_filename = filename.to_lowercase();
    let path = std::path::Path::new(&lower_filename);

    let extension = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext,
        None => return FileCategory::Other,
    };

    match extension {
        // Video containers
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "flv" | "wmv"
        | "m4v" | "3gp" | "ts" | "mts" | "m2ts" | "ogv" | "divx" => FileCategory::Video,
        // Image formats
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif"
        | "bmp" | "tiff" | "tif" | "svg" | "ico" | "avif" => FileCategory::Image,
        _ => FileCategory::Other,
    }
}

/// Optional: kiểm tra magic bytes để xác nhận file type thực sự
pub fn verify_magic_bytes(data: &[u8], claimed_category: FileCategory) -> bool {
    if data.len() < 4 {
        return false;
    }
    match claimed_category {
        FileCategory::Video => {
            // Check for common video signatures: ftyp (mp4/mov), 1A 45 DF A3 (webm/mkv), RIFF (avi)
            (data[4..8] == [0x66, 0x74, 0x79, 0x70]) // ftyp
                || (data[0..4] == [0x1A, 0x45, 0xDF, 0xA3]) // webm/mkv
                || (data[0..4] == [0x52, 0x49, 0x46, 0x46]) // RIFF (avi)
        }
        FileCategory::Image => {
            // JPEG: FF D8 FF, PNG: 89 50 4E 47, GIF: 47 49 46 38, WebP: 52 49 46 46...WEBP, BMP: 42 4D
            (data[0..3] == [0xFF, 0xD8, 0xFF]) // JPEG
                || (data[0..4] == [0x89, 0x50, 0x4E, 0x47]) // PNG
                || (data[0..4] == [0x47, 0x49, 0x46, 0x38]) // GIF
                || (data[0..4] == [0x52, 0x49, 0x46, 0x46]) // RIFF (WEBP)
                || (data[0..2] == [0x42, 0x4D]) // BMP
        }
        FileCategory::Other => true, // Không verify
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_file() {
        assert_eq!(classify_file("movie.mp4"), FileCategory::Video);
        assert_eq!(classify_file("CLIP.MKV"), FileCategory::Video);
        assert_eq!(classify_file("video.avi"), FileCategory::Video);
        assert_eq!(classify_file("video.m4v"), FileCategory::Video);
        assert_eq!(classify_file("PHOTO.JPEG"), FileCategory::Image);
        assert_eq!(classify_file("icon.ico"), FileCategory::Image);
        assert_eq!(classify_file("doc.bmp"), FileCategory::Image);
        assert_eq!(classify_file("document.pdf"), FileCategory::Other);
        assert_eq!(classify_file("no_extension"), FileCategory::Other);
        assert_eq!(classify_file("archive.zip"), FileCategory::Other);
        assert_eq!(classify_file("music.mp3"), FileCategory::Other);
    }

    #[test]
    fn test_magic_bytes() {
        // JPEG: FF D8 FF xx
        assert!(verify_magic_bytes(&[0xFF, 0xD8, 0xFF, 0xE0], FileCategory::Image));
        // PNG: 89 50 4E 47
        assert!(verify_magic_bytes(&[0x89, 0x50, 0x4E, 0x47], FileCategory::Image));
        // Not a JPEG (starts with 00)
        assert!(!verify_magic_bytes(&[0x00, 0x00, 0x00, 0x00], FileCategory::Image));
        // MP4 ftyp box
        let mut mp4_header = vec![0u8; 12];
        mp4_header[4..8].copy_from_slice(&[0x66, 0x74, 0x79, 0x70]); // "ftyp"
        assert!(verify_magic_bytes(&mp4_header, FileCategory::Video));
    }
}
