//! File identity, for recognising a file that has already been imported.
//!
//! The hash is taken over the file's bytes, so the same download imported twice — from a
//! different folder, or after a rename — is recognised as the same file. It is not a
//! security boundary; SHA-256 is used because it is a well-understood, collision-free
//! choice and hashing a book-sized file costs a few milliseconds.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Hex-encoded SHA-256 of `bytes`.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    Ok(hash_bytes(&bytes))
}

/// Truncated form for display, long enough to stay unambiguous in a library this size.
pub fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_distinguishes_content() {
        assert_eq!(hash_bytes(b"abc"), hash_bytes(b"abc"));
        assert_ne!(hash_bytes(b"abc"), hash_bytes(b"abd"));
    }

    #[test]
    fn hash_matches_the_known_sha256_of_abc() {
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identical_files_in_different_places_hash_the_same() {
        let dir = std::env::temp_dir().join("bookmarks-hash-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.html");
        let b = dir.join("b.html");
        std::fs::write(&a, b"<html>same</html>").unwrap();
        std::fs::write(&b, b"<html>same</html>").unwrap();
        assert_eq!(hash_file(&a).unwrap(), hash_file(&b).unwrap());

        // A rename must not change identity, which is the point of hashing content
        // rather than the path.
        let c = dir.join("renamed.html");
        std::fs::rename(&b, &c).unwrap();
        assert_eq!(hash_file(&a).unwrap(), hash_file(&c).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reports_the_path() {
        let err = hash_file(Path::new("does-not-exist-xyz.html")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does-not-exist-xyz.html"), "got: {msg}");
    }

    #[test]
    fn short_is_bounded() {
        let h = hash_bytes(b"x");
        assert_eq!(short(&h).len(), 12);
        assert_eq!(short("abc").len(), 3);
    }
}
