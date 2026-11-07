//! `cargo xtask ocr-bench [DIR] [--target <percent>] [--engine tesseract|paddle]`: Thai OCR
//! character error rate over a local corpus.
//!
//! `DIR` (default `tests/data/thai_ocr`) holds page images and their transcripts: for every
//! `*.png` with a sibling `*.txt` of the same stem, one engine reads the page.
//!
//! `--engine tesseract` (the default, and the behaviour before the flag existed) runs the
//! installed `tesseract` the way the app runs it — `tesseract <png> stdout -l tha+eng -c
//! tessedit_create_tsv=1` — with `TESSDATA_PREFIX` pointing at the bundled
//! `assets/models/tessdata` when that directory exists (the caller's environment is otherwise
//! left alone). The level-5 words of the TSV are joined with spaces.
//!
//! `--engine paddle` measures the app's own PaddleThai engine without linking it into xtask: it
//! builds a one-page PDF from the PNG and drives `pdfcraft-cli run --script` (the first document
//! of a fresh CLI session is id 1) through `doc_create`, `ocr_recognize` (`language` `th`,
//! `engine` `paddle`) and `doc_export_text`, then scores the exported text exactly like the
//! tesseract path. The CLI is `$PDFCRAFT_CLI` or `target/debug/pdfcraft-cli`; when the program,
//! its paddle engine or the three models under `assets/models` are missing the command prints a
//! "skipped" line and exits zero, like a missing tesseract.
//!
//! `--target` sets the pass mark in percent (default [`DEFAULT_TARGET`]) and may appear before or
//! after `DIR`; `--engine` may appear anywhere `--target` may.
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
//! zero with a "skipped" line when the chosen engine cannot run here: `tesseract` is not
//! installed, or the CLI, its paddle engine or the Paddle models are missing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;
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

/// The CLI the paddle path drives, relative to the repository root (`$PDFCRAFT_CLI` wins).
const CLI: &str = "target/debug/pdfcraft-cli";

/// The three Paddle models, as named in ATTRIBUTION.toml; all must be present to measure.
const PADDLE_MODELS: [&str; 3] = ["paddle-text-detection.onnx", "paddle-thai-recognition.onnx", "paddle-thai-dict.txt"];

/// Where the fetched models live, relative to the repository root.
const MODELS: &str = "assets/models";

/// Which reader a run measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Engine {
    /// The installed `tesseract` program, run exactly as the app runs it.
    #[default]
    Tesseract,
    /// The app's PaddleThai engine, driven through `pdfcraft-cli run --script`.
    Paddle,
}

impl Engine {
    /// The name the flag takes and a failure reports.
    fn name(self) -> &'static str {
        match self {
            Self::Tesseract => "tesseract",
            Self::Paddle => "paddle",
        }
    }

    /// Parse one `--engine` value; anything else is refused rather than quietly defaulted.
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tesseract" => Ok(Self::Tesseract),
            "paddle" => Ok(Self::Paddle),
            other => bail!("`{other}` is not an engine; expected tesseract or paddle"),
        }
    }
}

/// What the command line selected: where the corpus lives, the pass mark in percent and the
/// engine.
#[derive(Debug, PartialEq)]
struct Options {
    /// `None` means the default corpus under the repository root.
    dir: Option<PathBuf>,
    /// The total CER a run may reach, in percent.
    target: f64,
    /// The reader every page goes through.
    engine: Engine,
}

/// Why a page's reading failed. An engine that cannot run at all skips the whole command, like a
/// missing tesseract; a page that fails while the engine works is a measurement failure.
enum Failure {
    Unavailable(String),
    Failed(String),
}

pub fn run(args: &[String]) -> Result<()> {
    let root = crate::gates::root();
    let options = parse_args(args)?;
    let dir = options.dir.clone().unwrap_or_else(|| root.join(DEFAULT_DIR));
    let target = options.target / 100.0;
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    // Decide up front what reads the pages, and treat a reader that cannot run as the caller's
    // "skip" (zero exit), the same way a missing tesseract always has.
    let tessdata = root.join(TESSDATA);
    let tessdata = tessdata.is_dir().then_some(tessdata);
    let cli = match options.engine {
        Engine::Tesseract => {
            if !tesseract_installed()? {
                println!("skipped: tesseract is not installed");
                return Ok(());
            }
            None
        }
        Engine::Paddle => {
            let Some(cli) = cli_program(&root) else {
                println!(
                    "skipped: pdfcraft-cli/engine not available (no {}; set PDFCRAFT_CLI or build it with `cargo build -p pdfcraft-cli`)",
                    root.join(CLI).display()
                );
                return Ok(());
            };
            if paddle_models(&root).is_none() {
                println!(
                    "skipped: pdfcraft-cli/engine not available (the paddle models are missing under {}; run `cargo xtask models`)",
                    root.join(MODELS).display()
                );
                return Ok(());
            }
            if !paddle_engine_offered(&cli)? {
                println!("skipped: pdfcraft-cli/engine not available (the CLI's ocr_recognize has no paddle engine)");
                return Ok(());
            }
            Some(cli)
        }
    };

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
        let read = match (&options.engine, cli.as_deref()) {
            (Engine::Tesseract, _) => recognize(&png, tessdata.as_deref()),
            (Engine::Paddle, Some(cli)) => recognize_paddle(cli, &png),
            (Engine::Paddle, None) => Err(Failure::Unavailable("no pdfcraft-cli was chosen".into())),
        };
        let hypothesis = match read {
            Ok(text) => text,
            Err(Failure::Unavailable(note)) => {
                println!("skipped: pdfcraft-cli/engine not available: {note}");
                return Ok(());
            }
            Err(Failure::Failed(note)) => {
                println!("{name:<FILE_WIDTH$} {:>10} {:>7} {:>8}", "-", "-", "FAIL");
                eprintln!("ocr-bench: {name}: {} failed: {note}", options.engine.name());
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

/// Parse `[DIR]`, `--target <percent>` and `--engine <name>` in any order, rejecting anything
/// else.
fn parse_args(args: &[String]) -> Result<Options> {
    const USAGE: &str = "usage: cargo xtask ocr-bench [DIR] [--target <percent>] [--engine tesseract|paddle]";
    let mut dir = None;
    let mut target = None;
    let mut engine = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--target" {
            let value = rest.next().ok_or_else(|| anyhow!("--target needs a percentage; {USAGE}"))?;
            set_target(&mut target, value)?;
        } else if let Some(value) = arg.strip_prefix("--target=") {
            set_target(&mut target, value)?;
        } else if arg == "--engine" {
            let value = rest.next().ok_or_else(|| anyhow!("--engine needs a name; {USAGE}"))?;
            set_engine(&mut engine, value)?;
        } else if let Some(value) = arg.strip_prefix("--engine=") {
            set_engine(&mut engine, value)?;
        } else if arg.starts_with('-') && arg != "-" {
            bail!("unknown option `{arg}`; {USAGE}");
        } else if dir.is_some() {
            bail!("expected at most one DIR; {USAGE}");
        } else {
            dir = Some(PathBuf::from(arg));
        }
    }
    Ok(Options { dir, target: target.unwrap_or(DEFAULT_TARGET), engine: engine.unwrap_or_default() })
}

/// Record one `--engine` value, refusing a second one.
fn set_engine(engine: &mut Option<Engine>, value: &str) -> Result<()> {
    if engine.replace(Engine::parse(value)?).is_some() {
        bail!("--engine given more than once");
    }
    Ok(())
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

/// Run the installed `tesseract` on `png` exactly as the app does, and return its words joined
/// with spaces. The caller's environment is inherited; only `TESSDATA_PREFIX` is set when a
/// `tessdata` bundle exists.
fn recognize(png: &Path, tessdata: Option<&Path>) -> Result<String, Failure> {
    let mut command = Command::new("tesseract");
    command.arg(png).args(["stdout", "-l", LANGUAGES, "-c", "tessedit_create_tsv=1"]).stdin(std::process::Stdio::null());
    if let Some(dir) = tessdata {
        command.env("TESSDATA_PREFIX", dir);
    }
    let output = command.output().map_err(|err| Failure::Failed(format!("could not start tesseract: {err}")))?;
    if !output.status.success() {
        return Err(Failure::Failed(stderr_tail(&output.stderr)));
    }
    Ok(tsv_words(&String::from_utf8_lossy(&output.stdout)).join(" "))
}

/// The CLI the paddle path runs: `$PDFCRAFT_CLI` when set, else the debug build under the
/// repository root. A program that is not there is the caller's "skip".
fn cli_program(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PDFCRAFT_CLI") {
        return Some(PathBuf::from(path));
    }
    let path = root.join(CLI).with_extension(std::env::consts::EXE_EXTENSION);
    path.is_file().then_some(path)
}

/// The directory holding all three Paddle models: `$PDFCRAFT_MODELS` when it has them, else the
/// repository's `assets/models`.
fn paddle_models(root: &Path) -> Option<PathBuf> {
    let env = std::env::var_os("PDFCRAFT_MODELS").map(PathBuf::from);
    env.into_iter().chain([root.join(MODELS)]).find(|dir| PADDLE_MODELS.iter().all(|name| dir.join(name).is_file()))
}

/// Whether the CLI advertises `paddle` in `ocr_recognize`'s engine list. This asks the program
/// itself, so an older build without the engine skips instead of failing every page.
fn paddle_engine_offered(cli: &Path) -> Result<bool> {
    let output =
        Command::new(cli).arg("tools").stdin(std::process::Stdio::null()).output().with_context(|| format!("could not run `{}`", cli.display()))?;
    if !output.status.success() {
        bail!("`{} tools` failed: {}", cli.display(), stderr_tail(&output.stderr));
    }
    let tools: serde_json::Value = serde_json::from_slice(&output.stdout).with_context(|| format!("`{} tools` did not print JSON", cli.display()))?;
    let offered = tools.as_array().is_some_and(|tools| {
        tools.iter().any(|tool| {
            tool["name"] == "ocr_recognize"
                && tool["input_schema"]["properties"]["engine"]["enum"].as_array().is_some_and(|values| values.iter().any(|v| v == "paddle"))
        })
    });
    Ok(offered)
}

/// Build a one-page PDF from `png` with the CLI, recognise it with the Paddle engine and return
/// the exported text. The CLI runs as a child process, so xtask never links the OCR engine.
fn recognize_paddle(cli: &Path, png: &Path) -> Result<String, Failure> {
    let dir = temp_dir();
    let result = run_paddle_script(cli, png, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The body of [`recognize_paddle`]: write the script, run it, read the text it exported.
fn run_paddle_script(cli: &Path, png: &Path, dir: &Path) -> Result<String, Failure> {
    std::fs::create_dir_all(dir).map_err(|err| Failure::Failed(format!("could not make {}: {err}", dir.display())))?;
    let text = dir.join("page.txt");
    let script = dir.join("steps.json");
    // A fresh CLI session numbers its first document 1: doc_create makes it, and the two steps
    // after it name that id. --root is not needed; the paths are absolute.
    let steps = json!([
        { "tool": "doc_create", "args": { "from": "images", "paths": [png.to_string_lossy()] } },
        { "tool": "ocr_recognize", "args": { "doc": 1, "pages": [1], "language": "th", "engine": "paddle" } },
        { "tool": "doc_export_text", "args": { "doc": 1, "path": text.to_string_lossy() } },
    ]);
    std::fs::write(&script, serde_json::to_vec_pretty(&steps).map_err(|err| Failure::Failed(err.to_string()))?)
        .map_err(|err| Failure::Failed(format!("could not write {}: {err}", script.display())))?;
    let output = Command::new(cli)
        .args(["run", "--script"])
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| Failure::Failed(format!("could not start {}: {err}", cli.display())))?;
    if !output.status.success() {
        let note = stderr_tail(&output.stderr);
        return Err(if engine_unavailable(&note) { Failure::Unavailable(note) } else { Failure::Failed(note) });
    }
    std::fs::read_to_string(&text).map_err(|err| Failure::Failed(format!("the CLI wrote no {}: {err}", text.display())))
}

/// Whether a CLI error says the requested engine cannot run here, as opposed to a page that could
/// not be read. The tool refuses an engine it does not offer up front ("unsupported engine
/// ..."), and the engine names missing models when they cannot be loaded.
fn engine_unavailable(note: &str) -> bool {
    let note = note.to_ascii_lowercase();
    note.contains("unsupported engine")
        || note.contains("not available")
        || note.contains("no models")
        || (note.contains("models") && note.contains("not installed"))
}

/// A fresh, empty scratch directory for one page's script, named by process and a counter.
fn temp_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-bench-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    dir
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
    fn engine_defaults_to_tesseract() {
        let options = parse_args(&args(&[])).expect("no arguments");
        assert_eq!(options.engine, Engine::Tesseract);
        assert_eq!(options.engine.name(), "tesseract");
    }

    #[test]
    fn engine_flag_parses_both_values_in_any_order() {
        let default = parse_args(&args(&["pages", "--target", "12"])).expect("no engine");
        for spelling in ["tesseract", "TESSERACT", " tesseract "] {
            assert_eq!(parse_args(&args(&["--engine", spelling])).expect("tesseract").engine, Engine::Tesseract);
        }
        let before = parse_args(&args(&["--engine", "paddle", "pages", "--target", "12"])).expect("engine first");
        assert_eq!(before.engine, Engine::Paddle);
        assert_eq!(before.dir, default.dir);
        assert_eq!(before.target, default.target);
        let after = parse_args(&args(&["pages", "--target", "12", "--engine", "paddle"])).expect("engine last");
        assert_eq!(after, before);
        let equals = parse_args(&args(&["pages", "--engine=paddle"])).expect("--engine=value");
        assert_eq!(equals.engine, Engine::Paddle);
        assert_eq!(equals.dir, Some(PathBuf::from("pages")));
    }

    #[test]
    fn engine_rejects_what_is_not_an_engine() {
        assert!(parse_args(&args(&["--engine", "paddle "])).is_ok(), "surrounding space is trimmed");
        for value in ["ocrs", "auto", "", "PaddleThai", "paddle2"] {
            assert!(parse_args(&args(&["--engine", value])).is_err(), "{value:?} should be rejected");
        }
        assert!(parse_args(&args(&["--engine"])).is_err());
        assert!(parse_args(&args(&["--engine", "paddle", "--engine", "tesseract"])).is_err());
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
