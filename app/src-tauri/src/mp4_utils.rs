// ── Shared MP4 Box Parsing ─────────────────────────────────────────────
// Used by both transcode.rs (for source resolution detection) and
// commands/video_metadata.rs (for video metadata extraction).

/// Read a big-endian u32 at `offset` from `data`.
pub fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    let b = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Find a box by fourcc starting at `start_offset`, returning its end offset (start + size).
pub fn find_box(buffer: &[u8], start_offset: usize, fourcc: &[u8; 4]) -> Option<usize> {
    find_box_in_range(buffer, start_offset, buffer.len(), fourcc)
}

/// Find a box by fourcc within a given range, returning its end offset.
pub fn find_box_in_range(
    buffer: &[u8],
    start_offset: usize,
    range_end: usize,
    fourcc: &[u8; 4],
) -> Option<usize> {
    let mut offset = start_offset;
    while offset + 8 <= range_end {
        let size = match read_u32_be(buffer, offset) {
            Some(0) => break,
            Some(s) if s < 8 => break,
            Some(s) => s as usize,
            None => break,
        };
        if &buffer[offset + 4..offset + 8] == fourcc {
            return Some(offset + size);
        }
        offset += size;
    }
    None
}

/// Read the size of the box whose end is at `box_end`.
pub fn box_size_at(buffer: &[u8], box_end: usize) -> Option<usize> {
    let sz = read_u32_be(buffer, box_end.saturating_sub(8))?;
    if sz < 8 {
        None
    } else {
        Some(sz as usize)
    }
}

/// Check whether a trak box contains `vmhd` (walk trak → mdia → minf → vmhd).
pub fn trak_contains_vmhd(buffer: &[u8], trak_data_start: usize, trak_data_end: usize) -> bool {
    let mdia_end = match find_box_in_range(buffer, trak_data_start, trak_data_end, b"mdia") {
        Some(end) => end,
        None => return false,
    };
    let mdia_data_start = mdia_end.saturating_sub(box_size_at(buffer, mdia_end).unwrap_or(8)) + 8;

    let minf_end = match find_box_in_range(buffer, mdia_data_start, mdia_end, b"minf") {
        Some(end) => end,
        None => return false,
    };
    let minf_data_start = minf_end.saturating_sub(box_size_at(buffer, minf_end).unwrap_or(8)) + 8;

    find_box_in_range(buffer, minf_data_start, minf_end, b"vmhd").is_some()
}

/// Walk the moov box tree to find a video-track tkhd and extract display dimensions.
/// Returns `(width, height)` as 16.16 fixed-point values shifted right by 16.
pub fn scan_video_tkhd_dimensions(buffer: &[u8]) -> (Option<u32>, Option<u32>) {
    // Find the 'moov' box by searching from buffer start
    let moov_end = match find_box(buffer, 0, b"moov") {
        Some(e) => e,
        None => return (None, None),
    };
    let moov_start = moov_end.saturating_sub(box_size_at(buffer, moov_end).unwrap_or(0));

    // moov_start is the moov box start; its data begins at moov_start + 8
    let moov_data_end = moov_end;

    // Walk moov children looking for 'trak' boxes
    let mut pos = moov_start + 8;
    while pos + 8 < moov_data_end {
        let box_sz = match read_u32_be(buffer, pos) {
            Some(0) | None => break,
            Some(s) if s < 8 => break,
            Some(s) => s as usize,
        };

        if &buffer[pos + 4..pos + 8] == b"trak" {
            let trak_data_start = pos + 8;
            let trak_data_end = pos + box_sz;

            if trak_contains_vmhd(buffer, trak_data_start, trak_data_end) {
                // Video track — scan linearly inside for tkhd
                let mut tpos = trak_data_start;
                while tpos + 8 < trak_data_end {
                    let tsz = match read_u32_be(buffer, tpos) {
                        Some(0) => break,
                        Some(s) if s < 8 => break,
                        Some(s) => s as usize,
                        None => break,
                    };

                    if &buffer[tpos + 4..tpos + 8] == b"tkhd" {
                        // tkhd found at tpos; extract dimensions.
                        // Width/height are 16.16 fixed-point at the end of tkhd.
                        let version = buffer.get(tpos + 8).copied().unwrap_or(0);
                        let (w_off, h_off) = if version == 1 {
                            (tpos + 8 + 88, tpos + 8 + 92)
                        } else {
                            (tpos + 8 + 76, tpos + 8 + 80)
                        };

                        let width = read_u32_be(buffer, w_off).map(|w| w >> 16);
                        let height = read_u32_be(buffer, h_off).map(|h| h >> 16);
                        return (width, height);
                    }

                    tpos += tsz;
                }
            }
        }

        pos += box_sz;
    }

    (None, None)
}
