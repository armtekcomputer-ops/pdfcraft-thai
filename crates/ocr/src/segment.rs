//! Thai word boundaries.
//!
//! Thai writes words without spaces between them, so a run of recognised characters is not a
//! run of words: the dictionary segmenter (ICU4X) is what says where one word ends. Grouping
//! recognised text into words is what search, the per-language user dictionary and phrase
//! editing need; shaping and drawing never depend on it.
//!
//! Segmentation data exists only in desktop builds: the web build compiles the OCR engine out,
//! and [`words`] returns the whole string there rather than pull the dictionaries into wasm.

/// Whether `c` is a Thai character: U+0E00–U+0E7F covers the letters, the combining marks
/// (vowels and tone marks that stack above and below), the digits and Thai punctuation.
pub fn is_thai(c: char) -> bool {
    matches!(c, '\u{0E00}'..='\u{0E7F}')
}

/// Whether `text` contains any Thai character.
pub fn has_thai(text: &str) -> bool {
    text.chars().any(is_thai)
}

/// Whether `text` ends with a Thai character (a combining mark counts: it belongs to the
/// cluster before it).
pub fn ends_thai(text: &str) -> bool {
    text.chars().next_back().is_some_and(is_thai)
}

/// Whether `text` starts with a Thai character.
pub fn starts_thai(text: &str) -> bool {
    text.chars().next().is_some_and(is_thai)
}

/// Whether two adjacent words form one unspaced run: both Thai, and the box gap between them is
/// a cluster's distance rather than the width of a space. Thai does use spaces between phrases,
/// so the gap decides — not the script alone. A non-finite or zero line height keeps them
/// together, since there is no yardstick to measure against.
pub fn joins_without_space(prev: &str, next: &str, gap: f32, height: f32) -> bool {
    if !(ends_thai(prev) && starts_thai(next)) {
        return false;
    }
    if !height.is_finite() || height <= 0.0 {
        return true;
    }
    gap <= height * 0.6
}

/// The dictionary words of `text`. Runs of spaces, punctuation and non-Thai text come back as
/// their own pieces, in order, so joining the result rebuilds `text` exactly.
///
/// Without compiled data (wasm32) the whole string is one piece.
#[cfg(not(target_arch = "wasm32"))]
pub fn words(text: &str) -> Vec<&str> {
    use icu_segmenter::WordSegmenter;

    // Built per call: the segmenter wraps compiled data but is not `Send`, so a shared static
    // is not available. Construction only borrows the baked tables, and a line of OCR text
    // costs microseconds to segment.
    let segmenter = WordSegmenter::new_dictionary(Default::default());

    let mut out = Vec::new();
    let mut last = 0usize;
    for boundary in segmenter.segment_str(text) {
        if boundary > last {
            out.push(&text[last..boundary]);
            last = boundary;
        }
    }
    out
}

/// [`words`] without compiled data: the whole string, so callers still get a well-formed line.
#[cfg(target_arch = "wasm32")]
pub fn words(text: &str) -> Vec<&str> {
    if text.is_empty() { Vec::new() } else { vec![text] }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dictionary finds the words Thai readers would: no spaces, still four words.
    #[test]
    fn thai_runs_segment_into_words() {
        assert_eq!(words("สวัสดีครับภาษาไทย"), vec!["สวัสดี", "ครับ", "ภาษา", "ไทย"]);
        assert_eq!(words("การทดสอบครั้งที่หนึ่ง"), vec!["การ", "ทดสอบ", "ครั้ง", "ที่", "หนึ่ง"]);
    }

    /// Joining the pieces rebuilds the input exactly, spaces and Latin included.
    #[test]
    fn segmentation_preserves_the_text() {
        for text in ["ราคา 1,250 บาท (รวม VAT)", "PDF และ OCR ทำงานได้", "ไม่มีThai"] {
            assert_eq!(words(text).concat(), text, "{text}");
        }
    }

    /// Script predicates read the Thai block only; Latin and CJK stay outside it.
    #[test]
    fn thai_block_is_recognised() {
        assert!(is_thai('ก') && is_thai('์') && is_thai('๙'));
        assert!(!is_thai('a') && !is_thai('日') && !is_thai(' '));
        assert!(has_thai("OCR ไทย") && !has_thai("OCR only"));
        assert!(ends_thai("ครับ") && starts_thai("ไทย") && !ends_thai("PDF") && !starts_thai("PDF"));
    }
}
