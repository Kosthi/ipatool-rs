//! The binary patch Unicorn's Windows ARM64 build needs.
//!
//! Unicorn's AArch64 TCG backend was compiled against Windows' 32-bit `long`,
//! which truncated two 64-bit masks to 32 bits. The fix is to set the
//! register-width bit on the four instructions of each affected pair, turning
//! the 32-bit `and`/`mov` into their 64-bit forms.
//!
//! Only the exact build pinned in [`super::artifact`] is patched: the layout is
//! checked before anything is written, and an unexpected one is rejected rather
//! than patched approximately.

use crate::error::ClientError;

/// Occurrences of each pattern in the pinned DLL.
const EXPECTED_MATCHES: usize = 16;

/// Distance from the first instruction pair to the second.
const PAIR_SPACING: usize = 24;

const FIRST_PATTERN: [u8; 8] = [0xE8, 0x21, 0xCC, 0x1A, 0xE8, 0x03, 0x28, 0x2A];
const SECOND_PATTERN: [u8; 8] = [0xEF, 0x21, 0xC8, 0x1A, 0xEF, 0x03, 0x2F, 0x2A];

/// The bit that selects the 64-bit register form, in the top byte of each
/// little-endian AArch64 instruction word.
const SF_BIT: u8 = 0x80;

pub fn patch_windows_arm64_tcg_masks(image: &mut [u8]) -> Result<(), ClientError> {
    let first = offsets(image, &FIRST_PATTERN);
    let second = offsets(image, &SECOND_PATTERN);

    if first.len() != EXPECTED_MATCHES || second.len() != first.len() {
        return Err(ClientError::Sap(format!(
            "unexpected AArch64 TCG mask layout ({}/{} matches, expected {EXPECTED_MATCHES})",
            first.len(),
            second.len()
        )));
    }

    for (first, second) in first.iter().zip(&second) {
        if *second != first + PAIR_SPACING {
            return Err(ClientError::Sap(
                "unexpected AArch64 TCG mask instruction spacing".into(),
            ));
        }

        for offset in [first + 3, first + 7, second + 3, second + 7] {
            image[offset] |= SF_BIT;
        }
    }

    Ok(())
}

fn offsets(data: &[u8], pattern: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut start = 0;

    while let Some(position) = find(&data[start..], pattern) {
        let offset = start + position;
        out.push(offset);
        start = offset + pattern.len();
    }

    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One well-formed pair, repeated to the count the real DLL has.
    fn image() -> Vec<u8> {
        let mut out = Vec::new();

        for _ in 0..EXPECTED_MATCHES {
            let start = out.len();
            out.extend_from_slice(&FIRST_PATTERN);
            out.resize(start + PAIR_SPACING, 0);
            out.extend_from_slice(&SECOND_PATTERN);
            out.resize(start + PAIR_SPACING * 2, 0);
        }

        out
    }

    #[test]
    fn sets_the_register_width_bit_on_all_four_instructions() {
        let mut image = image();
        patch_windows_arm64_tcg_masks(&mut image).unwrap();

        assert_eq!(
            &image[0..8],
            &[0xE8, 0x21, 0xCC, 0x9A, 0xE8, 0x03, 0x28, 0xAA]
        );
        assert_eq!(
            &image[PAIR_SPACING..PAIR_SPACING + 8],
            &[0xEF, 0x21, 0xC8, 0x9A, 0xEF, 0x03, 0x2F, 0xAA]
        );
    }

    #[test]
    fn patching_is_idempotent() {
        let mut once = image();
        patch_windows_arm64_tcg_masks(&mut once).unwrap();

        // The patched bytes no longer match, so a second pass finds nothing.
        let mut twice = once.clone();
        assert!(patch_windows_arm64_tcg_masks(&mut twice).is_err());
    }

    #[test]
    fn rejects_a_different_build() {
        let mut image = image();
        image.truncate(PAIR_SPACING * 2);

        let error = patch_windows_arm64_tcg_masks(&mut image)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("unexpected AArch64 TCG mask layout"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unexpected_spacing() {
        let mut image = Vec::new();

        for _ in 0..EXPECTED_MATCHES {
            let start = image.len();
            image.extend_from_slice(&FIRST_PATTERN);
            // One word short of where the second half belongs.
            image.resize(start + PAIR_SPACING - 4, 0);
            image.extend_from_slice(&SECOND_PATTERN);
            image.resize(start + PAIR_SPACING * 2, 0);
        }

        let error = patch_windows_arm64_tcg_masks(&mut image)
            .unwrap_err()
            .to_string();

        assert!(error.contains("spacing"), "{error}");
    }

    #[test]
    fn finds_every_occurrence_without_overlapping() {
        let data = [1u8, 2, 1, 2, 1, 2];

        assert_eq!(offsets(&data, &[1, 2]), vec![0, 2, 4]);
        assert_eq!(offsets(&data, &[9]), Vec::<usize>::new());
    }
}
