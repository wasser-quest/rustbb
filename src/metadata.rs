//! Removes metadata from uploaded images: camera and location data (EXIF), XMP and IPTC blocks,
//! text chunks and comments. The picture data itself is copied byte for byte, so the image looks the
//! same. A file that cannot be read cleanly is refused rather than stored with its metadata.

const REFUSED: &str = "The picture could not be read. Save it again as PNG or JPEG and upload it again.";

/// The image without its metadata, for the content type decided by `upload::sniff`.
pub fn strip(bytes: &[u8], content_type: &str) -> Result<Vec<u8>, &'static str> {
    match content_type {
        "image/jpeg" => jpeg(bytes),
        "image/png" => png(bytes),
        "image/gif" => gif(bytes),
        "image/webp" => webp(bytes),
        _ => Err(REFUSED),
    }
}

/// JPEG segments: markers FF xx with a two-byte length, except the start, end and restart markers.
/// Kept: JFIF (APP0), colour profile (APP2) and Adobe colour data (APP14). Dropped: EXIF and XMP
/// (APP1), IPTC (APP13), the other application segments, and comments (COM).
fn jpeg(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(REFUSED);
    }
    let mut out = vec![0xFF, 0xD8];
    let mut pos = 2;
    while pos < bytes.len() {
        if bytes[pos] != 0xFF {
            return Err(REFUSED);
        }
        // Fill bytes (FF FF …) may precede a marker.
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        let Some(&marker) = bytes.get(pos) else { return Err(REFUSED) };
        pos += 1;
        match marker {
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                return Ok(out);
            }
            0xD0..=0xD7 | 0x01 => out.extend_from_slice(&[0xFF, marker]),
            0xDA => {
                // Start of the entropy-coded data: everything up to the end marker is picture data.
                let length = segment_length(bytes, pos)?;
                out.extend_from_slice(&[0xFF, 0xDA]);
                out.extend_from_slice(&bytes[pos..pos + length]);
                out.extend_from_slice(&bytes[pos + length..]);
                return Ok(out);
            }
            _ => {
                let length = segment_length(bytes, pos)?;
                // Frame and table markers (0xC0–0xCF), quantisation (DQT), restart interval (DRI),
                // JFIF (APP0), colour profile (APP2) and Adobe colour data (APP14).
                let keep = matches!(marker, 0xC0..=0xCF | 0xDB | 0xDD | 0xE0 | 0xE2 | 0xEE);
                if keep {
                    out.extend_from_slice(&[0xFF, marker]);
                    out.extend_from_slice(&bytes[pos..pos + length]);
                }
                pos += length;
            }
        }
    }
    Err(REFUSED)
}

/// The length of a JPEG segment starting at `pos` (the length field includes itself).
fn segment_length(bytes: &[u8], pos: usize) -> Result<usize, &'static str> {
    let high = *bytes.get(pos).ok_or(REFUSED)? as usize;
    let low = *bytes.get(pos + 1).ok_or(REFUSED)? as usize;
    let length = (high << 8) | low;
    if length < 2 || pos + length > bytes.len() {
        return Err(REFUSED);
    }
    Ok(length)
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// PNG chunks: length, type, data and CRC. Dropped: text (tEXt, zTXt, iTXt), EXIF (eXIf) and the
/// modification time (tIME). Every other chunk is copied with its CRC.
fn png(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err(REFUSED);
    }
    let mut out = PNG_SIGNATURE.to_vec();
    let mut pos = PNG_SIGNATURE.len();
    while pos < bytes.len() {
        let header = bytes.get(pos..pos + 8).ok_or(REFUSED)?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        let end = pos + 8 + length + 4;
        if end > bytes.len() {
            return Err(REFUSED);
        }
        let dropped = matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME");
        if !dropped {
            out.extend_from_slice(&bytes[pos..end]);
        }
        pos = end;
        if kind == b"IEND" {
            return Ok(out);
        }
    }
    Err(REFUSED)
}

/// GIF blocks: an image's data is copied, and only the graphic control and animation extensions are
/// kept. Comments, plain text and application data (for example XMP) are dropped.
fn gif(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 13 {
        return Err(REFUSED);
    }
    let flags = bytes[10];
    let table = if flags & 0x80 != 0 { 3 * (1usize << ((flags & 0x07) + 1)) } else { 0 };
    let header_end = 13 + table;
    let mut out = bytes.get(..header_end).ok_or(REFUSED)?.to_vec();
    let mut pos = header_end;
    loop {
        match *bytes.get(pos).ok_or(REFUSED)? {
            0x3B => {
                out.push(0x3B);
                return Ok(out);
            }
            0x21 => {
                let label = *bytes.get(pos + 1).ok_or(REFUSED)?;
                let start = pos;
                pos += 2;
                let keep_identity = label == 0xFF && {
                    let block_len = *bytes.get(pos).ok_or(REFUSED)? as usize;
                    let name = bytes.get(pos + 1..pos + 1 + block_len).ok_or(REFUSED)?;
                    name == b"NETSCAPE2.0" || name == b"ANIMEXTS1.0"
                };
                pos = skip_sub_blocks(bytes, pos)?;
                if label == 0xF9 || keep_identity {
                    out.extend_from_slice(&bytes[start..pos]);
                }
            }
            0x2C => {
                // Image descriptor, optional local colour table, LZW data.
                let start = pos;
                let local_flags = *bytes.get(pos + 9).ok_or(REFUSED)?;
                let local = if local_flags & 0x80 != 0 { 3 * (1usize << ((local_flags & 0x07) + 1)) } else { 0 };
                pos += 10 + local + 1;
                pos = skip_sub_blocks(bytes, pos)?;
                out.extend_from_slice(&bytes[start..pos]);
            }
            _ => return Err(REFUSED),
        }
    }
}

/// Moves past sub-blocks (a length byte and that many bytes) up to the terminating zero length.
fn skip_sub_blocks(bytes: &[u8], mut pos: usize) -> Result<usize, &'static str> {
    loop {
        let length = *bytes.get(pos).ok_or(REFUSED)? as usize;
        pos += 1;
        if length == 0 {
            return Ok(pos);
        }
        pos += length;
        if pos > bytes.len() {
            return Err(REFUSED);
        }
    }
}

/// WebP is a RIFF file of chunks. Dropped: EXIF and XMP. The flags that announce them are cleared.
fn webp(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(REFUSED);
    }
    let mut chunks = Vec::new();
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let kind = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let padded = size + (size & 1);
        let end = pos + 8 + padded;
        if end > bytes.len() {
            return Err(REFUSED);
        }
        if kind != b"EXIF" && kind != b"XMP " {
            let mut chunk = bytes[pos..end].to_vec();
            if kind == b"VP8X" && chunk.len() > 8 {
                // Flags: 0x04 XMP present, 0x08 EXIF present. Both are cleared with their chunks.
                chunk[8] &= !0x0C;
            }
            chunks.extend_from_slice(&chunk);
        }
        pos = end;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((4 + chunks.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunks);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with_exif() -> Vec<u8> {
        let mut file = vec![0xFF, 0xD8];
        // APP0 (JFIF), kept.
        file.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, b'J', b'F']);
        // APP1 with an EXIF block carrying a location string, dropped.
        let exif = b"Exif\0\0GPS 48.1372N 11.5756E";
        file.extend_from_slice(&[0xFF, 0xE1]);
        file.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        file.extend_from_slice(exif);
        // COM, dropped.
        file.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x05, b'h', b'i', b'!']);
        // Quantisation table (kept), then the picture data and the end marker.
        file.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x00, 0x01]);
        file.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x04, 0x01, 0x01, 0x12, 0x34, 0x56]);
        file.extend_from_slice(&[0xFF, 0xD9]);
        file
    }

    #[test]
    fn jpeg_loses_exif_and_comments_and_keeps_the_picture() {
        let cleaned = strip(&jpeg_with_exif(), "image/jpeg").unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"Exif"), "EXIF removed");
        assert!(!cleaned.windows(3).any(|w| w == b"GPS"), "location removed");
        assert!(!cleaned.windows(3).any(|w| w == b"hi!"), "comment removed");
        assert!(cleaned.windows(2).any(|w| w == [0xFF, 0xE0]), "JFIF kept");
        assert!(cleaned.ends_with(&[0x12, 0x34, 0x56, 0xFF, 0xD9]), "picture data and end kept");
        assert_eq!(&cleaned[..2], &[0xFF, 0xD8]);
    }

    fn png_chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]); // CRC (not checked here)
        chunk
    }

    #[test]
    fn png_loses_text_and_exif_chunks() {
        let mut file = PNG_SIGNATURE.to_vec();
        file.extend(png_chunk(b"IHDR", &[0; 13]));
        file.extend(png_chunk(b"tEXt", b"Author\0Marvin at home"));
        file.extend(png_chunk(b"eXIf", b"GPS 48.1372N"));
        file.extend(png_chunk(b"IDAT", &[1, 2, 3]));
        file.extend(png_chunk(b"IEND", &[]));
        let cleaned = strip(&file, "image/png").unwrap();
        assert!(!cleaned.windows(6).any(|w| w == b"Author"));
        assert!(!cleaned.windows(3).any(|w| w == b"GPS"));
        assert!(cleaned.windows(4).any(|w| w == b"IDAT"), "picture data kept");
        assert!(cleaned.ends_with(&png_chunk(b"IEND", &[])));
    }

    #[test]
    fn webp_loses_exif_and_clears_its_flag() {
        let mut chunks = Vec::new();
        // VP8X: flags byte with EXIF (0x08) and ICC (0x20) set.
        chunks.extend_from_slice(b"VP8X");
        chunks.extend_from_slice(&10u32.to_le_bytes());
        chunks.extend_from_slice(&[0x28, 0, 0, 0, 1, 0, 0, 1, 0, 0]);
        chunks.extend_from_slice(b"EXIF");
        chunks.extend_from_slice(&4u32.to_le_bytes());
        chunks.extend_from_slice(b"GPS!");
        chunks.extend_from_slice(b"VP8 ");
        chunks.extend_from_slice(&3u32.to_le_bytes());
        chunks.extend_from_slice(&[9, 9, 9, 0]);
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&((4 + chunks.len()) as u32).to_le_bytes());
        file.extend_from_slice(b"WEBP");
        file.extend_from_slice(&chunks);
        let cleaned = strip(&file, "image/webp").unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"EXIF"));
        assert!(cleaned.windows(4).any(|w| w == b"VP8 "));
        let vp8x = cleaned.windows(4).position(|w| w == b"VP8X").unwrap();
        assert_eq!(cleaned[vp8x + 8] & 0x08, 0, "EXIF flag cleared");
        assert_eq!(cleaned[vp8x + 8] & 0x20, 0x20, "colour profile flag kept");
        assert_eq!(u32::from_le_bytes([cleaned[4], cleaned[5], cleaned[6], cleaned[7]]) as usize, cleaned.len() - 8);
    }

    #[test]
    fn gif_keeps_the_picture_and_drops_comments() {
        let mut file = b"GIF89a".to_vec();
        file.extend_from_slice(&[1, 0, 1, 0, 0x00, 0, 0]); // logical screen, no colour table
        file.extend_from_slice(&[0x21, 0xFE, 4, b'h', b'i', b'!', b'!', 0]); // comment, dropped
        file.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]); // graphic control, kept
        file.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]); // image descriptor
        file.extend_from_slice(&[2, 2, 0x4C, 0x01, 0]); // LZW data
        file.push(0x3B);
        let cleaned = strip(&file, "image/gif").unwrap();
        assert!(!cleaned.windows(2).any(|w| w == [0x21, 0xFE]), "comment removed");
        assert!(cleaned.windows(2).any(|w| w == [0x21, 0xF9]), "graphic control kept");
        assert_eq!(*cleaned.last().unwrap(), 0x3B);
    }

    #[test]
    fn a_broken_file_is_refused() {
        assert!(strip(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x40], "image/jpeg").is_err());
        assert!(strip(&PNG_SIGNATURE, "image/png").is_err());
    }
}
