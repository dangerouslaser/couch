//! Manual pairing codes: the 11-digit (or 21-digit, with vendor and product)
//! number printed on a device and shown by other ecosystems when they share a
//! device. Only the first ten digits carry the discriminator and passcode; the
//! last is a Verhoeff check digit, which is verified here because a mistyped
//! code otherwise turns into a ten-second mDNS search that finds nothing.

use crate::Error;

/// Digits only, with a valid length and check digit. Dashes and spaces are
/// dropped so codes may be typed as printed (`3497-011-2332`).
pub fn normalize(code: &str) -> Result<String, Error> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code
        .chars()
        .any(|c| !c.is_ascii_digit() && !matches!(c, '-' | ' '))
    {
        return Err(Error::Invalid(
            "Enter the numeric pairing code, digits and dashes only".into(),
        ));
    }
    if digits.len() != 11 && digits.len() != 21 {
        return Err(Error::Invalid(
            "A pairing code has 11 digits, or 21 with vendor and product".into(),
        ));
    }
    if !verhoeff_valid(&digits) {
        return Err(Error::Invalid(
            "The pairing code has a typo; check its last digit".into(),
        ));
    }
    Ok(digits)
}

/// `3497-011-2332`, the layout the Matter specification prints.
pub fn display(digits: &str) -> String {
    if digits.len() < 11 {
        return digits.to_string();
    }
    let mut out = format!("{}-{}-{}", &digits[..4], &digits[4..7], &digits[7..11]);
    if digits.len() > 11 {
        out.push('-');
        out.push_str(&digits[11..]);
    }
    out
}

/// The Verhoeff check over every digit including the trailing check digit
/// evaluates to zero when the code is intact.
pub fn verhoeff_valid(digits: &str) -> bool {
    const D: [[u8; 10]; 10] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
        [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
        [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
        [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
        [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
        [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
        [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
        [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
        [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
    ];
    const P: [[u8; 10]; 8] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
        [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
        [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
        [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
        [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
        [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
        [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
    ];
    let mut c = 0u8;
    for (i, ch) in digits.bytes().rev().enumerate() {
        if !ch.is_ascii_digit() {
            return false;
        }
        c = D[c as usize][P[i % 8][(ch - b'0') as usize] as usize];
    }
    c == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    // The Matter SDK's test pairing code: discriminator 3840, passcode 20202021.
    const SDK_CODE: &str = "34970112332";

    #[test]
    fn sdk_test_code_passes_and_decodes_as_the_sdk_documents() {
        assert_eq!(normalize("3497-011-2332").unwrap(), SDK_CODE);
        assert_eq!(normalize("3497 011 2332").unwrap(), SDK_CODE);
        assert_eq!(display(SDK_CODE), "3497-011-2332");
        let info = matc::onboarding::decode_manual_pairing_code(SDK_CODE).unwrap();
        assert_eq!(info.passcode, 20202021);
        assert_eq!(info.discriminator & 0xF00, 3840 & 0xF00);
        assert!(info.is_short_discriminator);
    }

    #[test]
    fn typos_short_codes_and_letters_are_rejected_before_any_network_search() {
        for bad in [
            "34970112333",
            "3497011233",
            "",
            "34970112332a",
            "MT:Y.K90",
            "3497-011-233",
        ] {
            assert!(matches!(normalize(bad), Err(Error::Invalid(_))), "{bad:?}");
        }
        // Every single-digit substitution breaks the check digit.
        for i in 0..SDK_CODE.len() {
            let mut bytes = SDK_CODE.as_bytes().to_vec();
            bytes[i] = if bytes[i] == b'9' { b'0' } else { bytes[i] + 1 };
            assert!(!verhoeff_valid(std::str::from_utf8(&bytes).unwrap()));
        }
    }
}
