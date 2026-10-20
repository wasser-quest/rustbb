//! Attachment rules: only the image types below are accepted, decided by their first bytes
//! (not by the file name or the type the browser claims). SVG and HTML are never accepted,
//! because they can run script when opened.

pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_FILES_PER_POST: usize = 4;

/// The content type of an accepted image, from its magic bytes; `None` for anything else.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Checks size and type. Returns the content type to store and serve the file with.
pub fn check(bytes: &[u8]) -> Result<&'static str, &'static str> {
    if bytes.is_empty() {
        return Err("The file is empty.");
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err("The file is too large (at most 5 MB).");
    }
    sniff(bytes).ok_or("Only PNG, JPEG, GIF and WebP images are allowed.")
}

/// A random, unguessable id for the object and its public URL.
pub fn new_id() -> String {
    crate::auth::random_token()[..32].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_images_by_their_bytes() {
        assert_eq!(check(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]), Ok("image/png"));
        assert_eq!(check(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Ok("image/jpeg"));
        assert_eq!(check(b"GIF89a...."), Ok("image/gif"));
        assert_eq!(check(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Ok("image/webp"));
    }

    #[test]
    fn refuses_svg_html_scripts_and_renamed_files() {
        assert!(check(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>").is_err());
        assert!(check(b"<!doctype html><script>alert(1)</script>").is_err());
        assert!(check(b"MZ\x90\x00 executable").is_err());
        assert!(check(b"").is_err());
    }

    #[test]
    fn refuses_files_over_the_limit() {
        let big = vec![0x89u8; MAX_FILE_BYTES + 1];
        assert!(check(&big).is_err());
    }

    #[test]
    fn ids_are_unguessable_and_short() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
