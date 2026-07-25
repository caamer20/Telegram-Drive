#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Video,
    Image,
    Other,
}

pub fn classify_file(filename: &str) -> FileCategory {
    let lower_filename = filename.to_lowercase();
    let path = std::path::Path::new(&lower_filename);

    let extension = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext,
        None => return FileCategory::Other, // File không có extension
    };

    match extension {
        "mp4" | "mkv" | "mov" | "webm" => FileCategory::Video,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => FileCategory::Image,
        _ => FileCategory::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_file() {
        assert_eq!(classify_file("movie.mp4"), FileCategory::Video);
        assert_eq!(classify_file("CLIP.MKV"), FileCategory::Video);
        assert_eq!(classify_file("PHOTO.JPEG"), FileCategory::Image);
        assert_eq!(classify_file("document.pdf"), FileCategory::Other);
        assert_eq!(classify_file("no_extension"), FileCategory::Other);
        assert_eq!(classify_file("archive.zip"), FileCategory::Other);
    }
}
