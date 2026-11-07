//! `cargo xtask ocr-bench [DIR] [--target <percent>]`: Thai OCR character error rate over a local
//! corpus.
//!
//! `DIR` (default `tests/data/thai_ocr`) holds page images and their transcripts: for every
//! `*.png` with a sibling `*.txt` of the same stem, the installed `tesseract` runs the way the app
//! runs it — `tesseract <png> stdout -l tha+eng -c tessedit_create_tsv=1` — with `TESSDATA_PREFIX`
//! pointing at the bundled `assets/models/tessdata` when that directory exists (the caller's
//! environment is otherwise left alone). The level-5 words of the TSV are joined with spaces.
//! `--target` sets the pass mark in percent (default [`DEFAULT_TARGET`]) and may appear before or
//! after `DIR`.
//!
//! Both sides are normalised with Unicode NFKC before comparison. NFKC folds Thai SARA AM
//! (U+0E33) into NIKHAHIT (U+0E4D) plus SARA AA (U+0E32), the same sequence the recogniser emits;
//! without it a word like `น้ำ` would miss its `นํ้า` counterpart by two edits for the same
//! character. Because NIKHAHIT has combining class 0, NFKC leaves it after a preceding tone mark,
//! so a small final step moves it back in front — the order Thai writes and the recogniser emits.
//! Together they make a pre-composed and a decomposed transcript compare equal.
//!
//! Thai does not space every word, so joining with spaces could invent boundaries. The
//! comparison therefore removes every Unicode whitespace character from both the hypothesis and
//! the reference after NFKC: spacing choices cost nothing, and what is measured is the characters
//! themselves (a deliberate simplification of Thai CER; see `pdfcraft_fonts::segment` for the
//! app's own word-splitting rule). The score is Levenshtein distance over `char`s divided by the
//! reference length, per page and over the whole corpus (edits summed, then divided, so long
//! pages weigh more). A page whose recognition fails prints the tail of its stderr and is left
//! out of the totals, and any failure makes the command fail: a PASS over fewer pages would not
//! be the measurement it claims. A page above the target prints the start of its reference on a
//! short note, for orientation only.
//!
//! The command exits non-zero when the total CER exceeds the target, so CI can use it, and exits
//! zero with a "skipped" line when `tesseract` is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use unicode_normalization::UnicodeNormalization;

/// Where `cargo xtask ocr-bench` looks when no `DIR` is given, relative to the repository root.
const DEFAULT_DIR: &str = "tests/data/thai_ocr";

/// The bundled OCR data the app prefers, relative to the repository root; inherited
/// `TESSDATA_PREFIX` and the installed tree are used when it is absent.
const TESSDATA: &str = "assets/models/tessdata";

/// The largest total character error rate a corpus may have and still pass, in percent, when
/// `--target` is absent.
const DEFAULT_TARGET: f64 = 10.0;

/// How many reference characters an over-target note shows.
const PREVIEW_CHARS: usize = 40;

/// The language pair the app asks for.
const LANGUAGES: &str = "tha+eng";

/// Width of the file column in the results table.
const FILE_WIDTH: usize = 28;

/// How many trailing stderr lines a failure reports.
const STDERR_LINES: usize = 3;

/// The most characters of stderr a failure keeps.
const STDERR_CHARS: usize = 400;

/// What the command line selected: where the corpus lives and the pass mark in percent.
#[derive(Debug, PartialEq)]
struct Options {
    /// `None` means the default corpus under the repository root.
    dir: Option<PathBuf>,
    /// The total CER a run may reach, in percent.
    target: f64,
}

pub fn run(args: &[String]) -> Result<()> {
    let root = crate::gates::root();
    let options = parse_args(args)?;
    let dir = options.dir.unwrap_or_else(|| root.join(DEFAULT_DIR));
    let target = options.target / 100.0;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    if !tesseract_installed()? {
        println!("skipped: tesseract is not installed");
        return Ok(());
    }
    let tessdata = root.join(TESSDATA);
    let tessdata = tessdata.is_dir().then_some(tessdata);

    let pages = corpus(&dir)?;
    if pages.is_empty() {
        bail!("no *.png with a sibling *.txt in {}", dir.display());
    }
    let attempted = pages.len();

    println!("{:<FILE_WIDTH$} {:>10} {:>7} {:>8}", "file", "ref chars", "edits", "CER %");
    let mut measured = 0usize;
    let mut failures = 0usize;
    let mut total_chars = 0usize;
    let mut total_edits = 0usize;
    for png in pages {
        let name = file_name(&png);
        let reference = match std::fs::read_to_string(png.with_extension("txt")) {
            Ok(text) => text,
            Err(err) => {
                println!("{name:<FILE_WIDTH$} error: reading the reference failed: {err}");
                failures += 1;
                continue;
            }
        };
        let hypothesis = match recognize(&png, tessdata.as_deref()) {
            Ok(tsv) => tsv_words(&tsv).join(" "),
            Err(note) => {
                println!("{name:<FILE_WIDTH$} {:>10} {:>7} {:>8}", "-", "-", "FAIL");
                eprintln!("ocr-bench: {name}: tesseract failed: {note}");
                failures += 1;
                continue;
            }
        };
        let reference = normalise(&reference);
        let hypothesis = normalise(&hypothesis);
        let reference_chars = reference.chars().count();
        let edits = edit_distance(&reference, &hypothesis);
        let page_cer = cer(&reference, &hypothesis);
        println!("{name:<FILE_WIDTH$} {reference_chars:>10} {edits:>7} {:>8.2}", page_cer * 100.0);
        if page_cer > target {
            println!("  note: {name} is above target {:.2}%; reference starts \"{}\"", options.target, preview(&reference));
        }
        measured += 1;
        total_chars += reference_chars;
        total_edits += edits;
    }

    // An empty reference everywhere leaves the ratio undefined; zero edits is a clean pass.
    let total = if total_chars == 0 { if total_edits > 0 { 1.0 } else { 0.0 } } else { total_edits as f64 / total_chars as f64 };
    println!("{:<FILE_WIDTH$} {total_chars:>10} {total_edits:>7} {:>8.2}", "total", total * 100.0);
    let pass = total <= target;
    println!(
        "ocr-bench: {} (total CER {:.2}% {} target {:.2}%)",
        if pass { "PASS" } else { "FAIL" },
        total * 100.0,
        if pass { "<=" } else { ">" },
        options.target
    );

    if measured == 0 {
        bail!("no page could be recognised ({failures} of {attempted} failed)");
    }
    if failures > 0 {
        bail!("{failures} of {attempted} page(s) could not be measured");
    }
    if !pass {
        bail!("total CER {:.2}% exceeds the {:.2}% target", total * 100.0, options.target);
    }
    Ok(())
}

/// Parse `[DIR]` and `--target <percent>` in any order, rejecting anything else.
fn parse_args(args: &[String]) -> Result<Options> {
    const USAGE: &str = "usage: cargo xtask ocr-bench [DIR] [--target <percent>]";
    let mut dir = None;
    let mut target = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--target" {
            let value = rest.next().ok_or_else(|| anyhow!("--target needs a percentage; {USAGE}"))?;
            set_target(&mut target, value)?;
        } else if let Some(value) = arg.strip_prefix("--target=") {
            set_target(&mut target, value)?;
        } else if arg.starts_with('-') && arg != "-" {
            bail!("unknown option `{arg}`; {USAGE}");
        } else if dir.is_some() {
            bail!("expected at most one DIR; {USAGE}");
        } else {
            dir = Some(PathBuf::from(arg));
        }
    }
    Ok(Options { dir, target: target.unwrap_or(DEFAULT_TARGET) })
}

/// Record one `--target` value, refusing a second one.
fn set_target(target: &mut Option<f64>, value: &str) -> Result<()> {
    if target.replace(parse_target(value)?).is_some() {
        bail!("--target given more than once");
    }
    Ok(())
}

/// A percentage between 0 and 100, with an optional `%` suffix. Rejecting NaN and infinities keeps
/// the comparison and the printed summary meaningful.
fn parse_target(value: &str) -> Result<f64> {
    let percent =
        value.trim().trim_end_matches('%').parse::<f64>().map_err(|_| anyhow!("`{value}` is not a percentage; expected a number such as 10"))?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        bail!("--target {value} is out of range; expected 0 to 100 percent");
    }
    Ok(percent)
}

/// Whether the `tesseract` program answers. A missing program is the caller's "skip", any other
/// spawn failure is an error worth reporting.
fn tesseract_installed() -> Result<bool> {
    match Command::new("tesseract").arg("--version").output() {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(anyhow!("could not run `tesseract --version`: {err}")),
    }
}

/// The PNGs in `dir` that have a sibling `.txt` of the same stem, sorted by path.
fn corpus(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut pages = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let is_png = path.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("png"));
        if is_png && path.with_extension("txt").is_file() {
            pages.push(path);
        }
    }
    pages.sort();
    Ok(pages)
}

/// A short display name for a page file.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Run the installed `tesseract` on `png` exactly as the app does, and return its TSV on stdout.
/// The caller's environment is inherited; only `TESSDATA_PREFIX` is set when a `tessdata` bundle
/// exists.
fn recognize(png: &Path, tessdata: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("tesseract");
    command.arg(png).args(["stdout", "-l", LANGUAGES, "-c", "tessedit_create_tsv=1"]).stdin(std::process::Stdio::null());
    if let Some(dir) = tessdata {
        command.env("TESSDATA_PREFIX", dir);
    }
    let output = command.output().map_err(|err| format!("could not start tesseract: {err}"))?;
    if !output.status.success() {
        return Err(stderr_tail(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The end of what the program wrote to stderr, on one line and bounded in length.
fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if lines.len() > STDERR_LINES {
        lines.drain(..lines.len() - STDERR_LINES);
    }
    let joined = if lines.is_empty() { "no error message".to_owned() } else { lines.join(" | ") };
    let start = joined.char_indices().rev().nth(STDERR_CHARS - 1).map_or(0, |(index, _)| index);
    joined.get(start..).unwrap_or_default().to_owned()
}

/// Fold both sides to NFKC and remove every Unicode whitespace character. NFKC turns SARA AM into
/// NIKHAHIT plus SARA AA, which is what the recogniser writes, so Thai word spacing and a
/// pre-composed versus decomposed spelling cannot change the score.
fn normalise(text: &str) -> String {
    let mut chars: Vec<char> = text.nfkc().filter(|c| !c.is_whitespace()).collect();
    reorder_nikhahit(&mut chars);
    chars.into_iter().collect()
}

/// Thai tone marks: MAI EK through MAI CHATTAWA.
fn is_tone_mark(c: char) -> bool {
    ('\u{0E48}'..='\u{0E4B}').contains(&c)
}

/// Move NIKHAHIT (U+0E4D) before the tone marks it follows. NFKC gives NIKHAHIT combining class 0,
/// so canonical ordering leaves `น้ำ` as tone mark then NIKHAHIT while a decomposed `นํ้า` already
/// has NIKHAHIT first; Thai (and the recogniser) write NIKHAHIT first. Without this the two
/// spellings would differ by two edits.
fn reorder_nikhahit(chars: &mut [char]) {
    for index in 1..chars.len() {
        if chars.get(index) != Some(&'\u{0E4D}') {
            continue;
        }
        let mut at = index;
        while at > 0 && chars.get(at - 1).is_some_and(|c| is_tone_mark(*c)) {
            chars.swap(at - 1, at);
            at -= 1;
        }
    }
}

/// The first [`PREVIEW_CHARS`] characters of a reference, with an ellipsis when there is more.
/// Notes show this much so an over-target page can be found without dumping the transcript.
fn preview(text: &str) -> String {
    let mut preview: String = text.chars().take(PREVIEW_CHARS).collect();
    if text.chars().nth(PREVIEW_CHARS).is_some() {
        preview.push('…');
    }
    preview
}

/// The Levenshtein distance between two strings, counted in `char`s.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }
    // Two rolling rows: the previous distances and the current one.
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (i, a) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, b) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(a != b);
            let insertion = current[j] + 1;
            let deletion = previous[j + 1] + 1;
            current[j + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// The character error rate of `hypothesis` against `reference`: edits divided by reference
/// length, after [`normalise`]. An empty reference scores 0 when the hypothesis is empty too, and
/// 1 otherwise, because the ratio itself is undefined.
fn cer(reference: &str, hypothesis: &str) -> f64 {
    let reference = normalise(reference);
    let hypothesis = normalise(hypothesis);
    let reference_chars = reference.chars().count();
    let edits = edit_distance(&reference, &hypothesis);
    if reference_chars == 0 {
        return if edits > 0 { 1.0 } else { 0.0 };
    }
    edits as f64 / reference_chars as f64
}

/// The words of a Tesseract TSV: level-5 rows, the twelfth column (index 11, `text`), with empty
/// cells dropped. The first line is the header.
fn tsv_words(tsv: &str) -> Vec<&str> {
    let mut words = Vec::new();
    for line in tsv.lines() {
        let mut columns = line.split('\t');
        if columns.next().is_none_or(|level| level.trim() != "5") {
            continue;
        }
        if let Some(word) = columns.nth(10) {
            let word = word.trim();
            if !word.is_empty() {
                words.push(word);
            }
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn normalise_removes_every_kind_of_whitespace() {
        assert_eq!(normalise(" ก \tข\nค\u{a0}ง "), "กขคง");
    }

    #[test]
    fn nfkc_folds_sara_am_to_nikhahit_and_sara_aa() {
        // Pre-composed SARA AM (U+0E33) against the decomposed spelling the recogniser emits.
        let composed = normalise("น้ำ");
        let decomposed = normalise("นํ้า");
        assert_eq!(composed, decomposed);
        // น + ํ + ้ + า: SARA AM became two characters.
        assert_eq!(composed.chars().count(), 4);
        assert_eq!(edit_distance(&composed, &decomposed), 0);
        assert_eq!(cer("น้ำ", "นํ้า"), 0.0);
    }

    #[test]
    fn target_defaults_to_ten_percent() {
        let options = parse_args(&args(&[])).expect("no arguments");
        assert_eq!(options.target, DEFAULT_TARGET);
        assert_eq!(options.dir, None);
        let options = parse_args(&args(&["pages"])).expect("a directory");
        assert_eq!(options.dir, Some(PathBuf::from("pages")));
    }

    #[test]
    fn target_flag_parses_in_any_order() {
        let before = parse_args(&args(&["--target", "15", "pages"])).expect("target before the directory");
        assert_eq!(before.target, 15.0);
        assert_eq!(before.dir, Some(PathBuf::from("pages")));
        let after = parse_args(&args(&["pages", "--target", "15"])).expect("target after the directory");
        assert_eq!(after, before);
    }

    #[test]
    fn target_accepts_a_percent_suffix_and_both_bounds() {
        assert_eq!(parse_target("12.5%").expect("a percentage"), 12.5);
        assert_eq!(parse_target("0").expect("zero"), 0.0);
        assert_eq!(parse_target("100").expect("one hundred"), 100.0);
    }

    #[test]
    fn target_rejects_what_is_not_a_number() {
        assert!(parse_args(&args(&["--target", "ten"])).is_err());
        assert!(parse_args(&args(&["--target"])).is_err());
        assert!(parse_args(&args(&["--bogus", "10"])).is_err());
        assert!(parse_args(&args(&["--target", "10", "--target", "20"])).is_err());
    }

    #[test]
    fn target_rejects_out_of_range_values() {
        for value in ["-0.1", "100.1", "150", "NaN", "inf"] {
            assert!(parse_target(value).is_err(), "{value} should be rejected");
        }
    }

    #[test]
    fn preview_shows_at_most_forty_characters() {
        assert_eq!(preview("สั้น"), "สั้น");
        let long = "ก".repeat(PREVIEW_CHARS + 5);
        let expected = format!("{}…", "ก".repeat(PREVIEW_CHARS));
        assert_eq!(preview(&long), expected);
    }

    #[test]
    fn identical_strings_have_zero_edits() {
        assert_eq!(edit_distance("ภาษาไทย", "ภาษาไทย"), 0);
        assert_eq!(cer("ภาษาไทย", "ภาษาไทย"), 0.0);
    }

    #[test]
    fn one_substitution_costs_one_edit() {
        assert_eq!(edit_distance("abc", "abd"), 1);
        assert_eq!(cer("abcd", "abxd"), 0.25);
    }

    #[test]
    fn one_insertion_costs_one_edit() {
        assert_eq!(edit_distance("abc", "abxc"), 1);
    }

    #[test]
    fn one_deletion_costs_one_edit() {
        assert_eq!(edit_distance("abc", "ac"), 1);
    }

    #[test]
    fn whitespace_is_ignored_on_both_sides() {
        assert_eq!(cer("สวัสดี โลก", "สวัสดีโลก"), 0.0);
        assert_eq!(cer("สวัสดี\tโลก\n", " สวัสดี  โลก "), 0.0);
    }

    #[test]
    fn empty_reference_is_guarded() {
        assert_eq!(cer("", ""), 0.0);
        assert_eq!(cer("  ", ""), 0.0);
        assert_eq!(cer("", "x"), 1.0);
    }

    #[test]
    fn tsv_keeps_level_five_words_only() {
        let tsv = concat!(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n",
            "1\t1\t0\t0\t0\t0\t0\t0\t100\t20\t-1\t\n",
            "5\t1\t1\t1\t1\t1\t10\t5\t20\t10\t96\tสวัสดี\n",
            "5\t1\t1\t1\t1\t2\t35\t5\t20\t10\t94\tโลก\n",
        );
        let words: Vec<&str> = tsv_words(tsv);
        assert_eq!(words, vec!["สวัสดี", "โลก"]);
    }
}
