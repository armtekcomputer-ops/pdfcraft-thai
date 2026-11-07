//! The PaddleOCR engine: Thai text through the PP-OCR detection and recognition models
//! (Apache-2.0; see ATTRIBUTION.toml), wrapped as a [`Recognizer`] (`PaddleRecognizer`).
//!
//! The pipeline is the reference one: a DB-style detection model turns the page into a
//! probability map, connected boxes above a threshold become crop rectangles (expanded by
//! the Paddle `unclip` offset), and a CTC recognition model reads each crop into Thai text.
//! Each word's confidence is the decoder's own: the mean probability of the classes the
//! greedy CTC decode emitted, on the app's 0–100 scale (see `ctc_decode_with_score`).
//!
//! Both models are ONNX files run through `rten`; nothing leaves the machine.

use std::path::{Path, PathBuf};

use rten::{Model, ValueView};

use super::{Line, MAX_PIXELS, OcrError, OcrImage, RecognizeOptions, Recognizer, Word};

/// The model files, as named in ATTRIBUTION.toml.
pub const DETECTION_MODEL: &str = "paddle-text-detection.onnx";
pub const RECOGNITION_MODEL: &str = "paddle-thai-recognition.onnx";
pub const DICT: &str = "paddle-thai-dict.txt";

/// The languages this engine reads: the Thai recognition model's alphabet.
pub const LANGUAGES: &[(&str, &str)] = &[("th", "Thai")];

/// Detection input: the longer side is scaled down to this many pixels, both sides are
/// rounded to a multiple of 32 (the model's stride), and the short side never drops below 32.
const DETECT_MAX_SIDE: f32 = 960.0;
const DETECT_STRIDE: f32 = 32.0;
/// The ImageNet statistics PP-OCR normalises RGB with.
const DETECT_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const DETECT_STD: [f32; 3] = [0.229, 0.224, 0.225];
/// Probability above which a map pixel is text, and the mean probability a whole box needs.
const BOX_THRESHOLD: f32 = 0.3;
const BOX_SCORE: f32 = 0.6;
/// A box smaller than this many pixels on either side (in the resized map) is noise.
const MIN_BOX_SIDE: u32 = 3;
/// The Paddle expansion: `distance = area * ratio / perimeter`, applied on every side.
const UNCLIP_RATIO: f32 = 1.5;
/// Recognition input: the crop is scaled to this height. The width follows the box: the model
/// takes variable width, so a long line is fed at its own width rather than squashed into the
/// 320 pixels PaddleOCR uses as a batch floor. [`REC_HARD_MAX_WIDTH`] bounds the allocation for
/// a hostile box.
const REC_HEIGHT: u32 = 48;
const REC_HARD_MAX_WIDTH: u32 = 4096;
/// The recognition model's normalisation, `(v / 255 - 0.5) / 0.5`, as mean and std.
const REC_MEAN: [f32; 3] = [0.5; 3];
const REC_STD: [f32; 3] = [0.5; 3];

/// Where the three model files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaddleModels {
    pub detection: PathBuf,
    pub recognition: PathBuf,
    pub dict: PathBuf,
}

impl PaddleModels {
    /// `models/` beside the executable, `Resources/models` in a macOS bundle, and the
    /// `share/pdfcraft/models` of the Linux, BSD and Flatpak packages.
    pub fn dirs_beside_exe(exe_dir: &Path) -> Vec<PathBuf> {
        vec![exe_dir.join("models"), exe_dir.join("../Resources/models"), exe_dir.join("../share/pdfcraft/models")]
    }

    /// The models in `dir`, if all three files are there.
    pub fn in_dir(dir: &Path) -> Option<PaddleModels> {
        let m = PaddleModels { detection: dir.join(DETECTION_MODEL), recognition: dir.join(RECOGNITION_MODEL), dict: dir.join(DICT) };
        (m.detection.is_file() && m.recognition.is_file() && m.dict.is_file()).then_some(m)
    }

    /// Look for the models: `$PDFCRAFT_MODELS`, then beside the executable (and
    /// `Resources/models` in a macOS bundle), then the source tree's `assets/models/`.
    pub fn find() -> Option<PaddleModels> {
        Self::search_dirs().iter().find_map(|d| Self::in_dir(d))
    }

    /// The directories [`PaddleModels::find`] looks in, in order.
    pub fn search_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(d) = std::env::var_os("PDFCRAFT_MODELS") {
            dirs.push(PathBuf::from(d));
        }
        if let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            dirs.extend(Self::dirs_beside_exe(&exe));
        }
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"));
        dirs
    }
}

/// A loaded recogniser. Loading takes a moment; keep one and reuse it.
pub struct PaddleRecognizer {
    detection: Model,
    recognition: Model,
    /// The alphabet: class `i > 0` maps to `dict[i - 1]` (plus a trailing space class some
    /// exports append).
    dict: Vec<String>,
}

impl PaddleRecognizer {
    /// Load the models and the alphabet from `m`.
    pub fn load(m: &PaddleModels) -> Result<PaddleRecognizer, OcrError> {
        let load = |p: &Path| Model::load_file(p).map_err(|e| OcrError::Load(p.display().to_string(), e.to_string()));
        let dict = std::fs::read_to_string(&m.dict).map_err(|e| OcrError::Load(m.dict.display().to_string(), e.to_string()))?;
        let dict: Vec<String> = dict.lines().map(str::to_string).collect();
        if dict.is_empty() {
            return Err(OcrError::Load(m.dict.display().to_string(), "the alphabet is empty".into()));
        }
        Ok(PaddleRecognizer { detection: load(&m.detection)?, recognition: load(&m.recognition)?, dict })
    }

    /// Load the models found by [`PaddleModels::find`].
    pub fn find() -> Result<PaddleRecognizer, OcrError> {
        Self::load(&PaddleModels::find().ok_or(OcrError::NoModels)?)
    }

    /// Run the detector and read every box it finds.
    fn read(&self, image: &OcrImage) -> Result<Vec<Word>, OcrError> {
        let (dw, dh) = detect_size(image.width(), image.height());
        let (ow, oh) = (image.width(), image.height());
        // Resize RGBA, then drop alpha: the detector reads RGB only.
        let resized = resize_bilinear(image.rgba(), 4, ow, oh, dw, dh);
        if resized.len() != (dw as usize * dh as usize).saturating_mul(4) {
            return Err(OcrError::Recognize("paddle: the detector input could not be resized".into()));
        }
        let rgb: Vec<u8> = resized.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let data = normalise(&rgb, dw, dh, DETECT_MEAN, DETECT_STD);
        let (mw, mh, probs) = self.run_detection(&data, dw, dh)?;

        let sx = ow as f32 / mw as f32;
        let sy = oh as f32 / mh as f32;
        let mut words = Vec::new();
        for component in connected_boxes(&probs, mw, mh, BOX_THRESHOLD) {
            let [l, t, r, b] = component.rect;
            if !component.mean.is_finite() || component.mean < BOX_SCORE || r.saturating_sub(l) < MIN_BOX_SIDE || b.saturating_sub(t) < MIN_BOX_SIDE {
                continue;
            }
            let expanded = unclip([l as f32, t as f32, r as f32, b as f32], UNCLIP_RATIO);
            let Some(rect) = clamp_box([expanded[0] * sx, expanded[1] * sy, expanded[2] * sx, expanded[3] * sy], ow, oh) else {
                continue;
            };
            let Some((cw, ch, crop)) = crop_rgb(image, rect) else { continue };
            let (text, confidence) = self.read_box(&crop, cw, ch)?;
            if !text.trim().is_empty() {
                words.push(Word::new(text, rect, "paddle").with_confidence(confidence * 100.0));
            }
        }
        Ok(words)
    }

    /// The detector's probability map as `(width, height, pixels)`, from a normalised NCHW
    /// input.
    fn run_detection(&self, data: &[f32], width: u32, height: u32) -> Result<(u32, u32, Vec<f32>), OcrError> {
        let (w, h) = (width as usize, height as usize);
        let input = ValueView::from_shape([1, 3, h, w], data).map_err(|e| OcrError::Recognize(format!("paddle detection input: {e}")))?;
        let out = self.detection.run_one(input.into(), None).map_err(|e| OcrError::Recognize(format!("paddle detection: {e}")))?;
        let (shape, probs) = out.into_shape_vec::<f32, 4>().map_err(|e| OcrError::Recognize(format!("paddle detection output: {e}")))?;
        let [_, _, ph, pw] = shape;
        match pw.checked_mul(ph) {
            Some(pixels) if pw > 0 && ph > 0 && probs.len() == pixels => Ok((pw as u32, ph as u32, probs)),
            _ => Err(OcrError::Recognize(format!("paddle detection output has an unexpected shape {shape:?}"))),
        }
    }

    /// Read one crop (RGB, `cw` × `ch`) through the recognition model and decode its CTC
    /// output, returning the text and the decoder's confidence (0–1).
    fn read_box(&self, crop: &[u8], cw: u32, ch: u32) -> Result<(String, f32), OcrError> {
        let rw = rec_width(cw, ch);
        let resized = resize_bilinear(crop, 3, cw, ch, rw, REC_HEIGHT);
        if resized.len() != rw as usize * REC_HEIGHT as usize * 3 {
            return Err(OcrError::Recognize("paddle: the recognition input could not be resized".into()));
        }
        let data = normalise(&resized, rw, REC_HEIGHT, REC_MEAN, REC_STD);
        let input = ValueView::from_shape([1, 3, REC_HEIGHT as usize, rw as usize], &data)
            .map_err(|e| OcrError::Recognize(format!("paddle recognition input: {e}")))?;
        let out = self.recognition.run_one(input.into(), None).map_err(|e| OcrError::Recognize(format!("paddle recognition: {e}")))?;
        let (shape, logits) = out.into_shape_vec::<f32, 3>().map_err(|e| OcrError::Recognize(format!("paddle recognition output: {e}")))?;
        let [_, steps, classes] = shape;
        Ok(ctc_decode_with_score(&logits, steps, classes, &self.dict))
    }
}

impl Recognizer for PaddleRecognizer {
    fn id(&self) -> &'static str {
        "paddle"
    }

    fn available(&self) -> bool {
        PaddleModels::find().is_some()
    }

    fn languages(&self) -> Vec<(String, String)> {
        LANGUAGES.iter().map(|(c, n)| ((*c).to_string(), (*n).to_string())).collect()
    }

    fn recognize(&self, image: &OcrImage, _options: &RecognizeOptions) -> Result<Vec<Line>, OcrError> {
        if image.width() == 0 || image.height() == 0 {
            return Err(OcrError::EmptyImage);
        }
        Ok(group_lines(self.read(image)?))
    }
}

/// The detector's input size for a `w` × `h` page: scale the longer side to at most
/// [`DETECT_MAX_SIDE`], round each side to a multiple of 32 (at least 32).
fn detect_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h).max(1) as f32;
    let scale = (DETECT_MAX_SIDE / long).min(1.0);
    let side = |dim: u32| -> u32 {
        let scaled = (dim as f32 * scale / DETECT_STRIDE).round() * DETECT_STRIDE;
        (scaled.max(DETECT_STRIDE)) as u32
    };
    (side(w), side(h))
}

/// The recognition input width for a `w` × `h` crop: the aspect-preserving `round(w * 48 / h)`,
/// at least one pixel. The model takes variable width, so a long line is fed at its own width
/// rather than squashed into [`REC_MAX_WIDTH`] (PaddleOCR's `max_wh_ratio` does the same: 320
/// is the floor of the padded batch width, never a cap on the content). The hard cap bounds
/// the allocation for a hostile box.
fn rec_width(w: u32, h: u32) -> u32 {
    if w == 0 || h == 0 {
        return 1;
    }
    ((w as f32 * REC_HEIGHT as f32 / h as f32).round() as u32).clamp(1, REC_HARD_MAX_WIDTH)
}

/// Bilinear resize of an interleaved `channels`-byte image, row-major. Returns an empty
/// vector when a side is zero, the buffers are inconsistent, or the result would exceed the
/// crate's area cap: callers treat that as a failure, never a panic.
pub(crate) fn resize_bilinear(src: &[u8], channels: usize, src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (src_w as usize, src_h as usize, dst_w as usize, dst_h as usize);
    if channels == 0 || sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Vec::new();
    }
    let Some(dst_pixels) = dw.checked_mul(dh) else { return Vec::new() };
    if dst_pixels > MAX_PIXELS as usize {
        return Vec::new();
    }
    let Some(src_len) = sw.checked_mul(sh).and_then(|p| p.checked_mul(channels)) else { return Vec::new() };
    let Some(dst_len) = dst_pixels.checked_mul(channels) else { return Vec::new() };
    if src.len() < src_len {
        return Vec::new();
    }
    let (x_scale, y_scale) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    let mut out = vec![0u8; dst_len];
    // Sample positions at pixel centres, clamped to the edge: the same convention PP-OCR's
    // cv2.resize uses with `INTER_LINEAR`.
    let at = |x: usize, y: usize, ch: usize| -> f32 { src.get(y * sw * channels + x * channels + ch).copied().unwrap_or(0) as f32 };
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * y_scale - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * x_scale - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            let o = (y * dw + x) * channels;
            for ch in 0..channels {
                let top = at(x0, y0, ch) * (1.0 - fx) + at(x1, y0, ch) * fx;
                let bottom = at(x0, y1, ch) * (1.0 - fx) + at(x1, y1, ch) * fx;
                let value = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
                if let Some(slot) = out.get_mut(o + ch) {
                    *slot = value;
                }
            }
        }
    }
    out
}

/// An interleaved RGB image as normalised NCHW `f32`, `(v / 255 - mean) / std` per channel.
/// Empty when the buffer does not hold exactly `width * height * 3` bytes, a dimension is
/// zero, or a standard deviation is zero or not finite.
pub(crate) fn normalise(rgb: &[u8], width: u32, height: u32, mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    if std.iter().any(|s| !s.is_finite() || *s == 0.0) || mean.iter().any(|m| !m.is_finite()) {
        return Vec::new();
    }
    let Some(pixels) = w.checked_mul(h) else { return Vec::new() };
    if pixels > MAX_PIXELS as usize {
        return Vec::new();
    }
    let Some(len) = pixels.checked_mul(3) else { return Vec::new() };
    if rgb.len() < len {
        return Vec::new();
    }
    let mut out = vec![0.0f32; len];
    for ch in 0..3 {
        let (m, s) = (mean[ch], std[ch]);
        for i in 0..pixels {
            let Some(v) = rgb.get(i * 3 + ch) else { break };
            if let Some(slot) = out.get_mut(ch * pixels + i) {
                *slot = (*v as f32 / 255.0 - m) / s;
            }
        }
    }
    out
}

/// One connected region of the probability map: its pixel box (`right` and `bottom`
/// exclusive) and the mean probability over its pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Component {
    pub rect: [u32; 4],
    pub mean: f32,
}

/// Find the 4-connected regions of `probs` at or above `threshold`, in raster order. `probs`
/// is a `width` × `height` row-major map. A buffer that does not match the dimensions yields
/// no regions.
pub(crate) fn connected_boxes(probs: &[f32], width: u32, height: u32, threshold: f32) -> Vec<Component> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let Some(len) = w.checked_mul(h) else { return Vec::new() };
    if probs.len() < len || !threshold.is_finite() {
        return Vec::new();
    }
    let mut seen = vec![false; len];
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for start in 0..len {
        if seen.get(start).copied().unwrap_or(true) || !probs.get(start).is_some_and(|&p| p >= threshold) {
            continue;
        }
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start);
        let (mut l, mut t, mut r, mut b) = (start % w, start / w, start % w + 1, start / w + 1);
        let (mut sum, mut count) = (0.0f64, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            l = l.min(x);
            t = t.min(y);
            r = r.max(x + 1);
            b = b.max(y + 1);
            if let Some(&p) = probs.get(i) {
                sum += f64::from(p);
                count += 1;
            }
            // Four-neighbour connectivity, as the reference's connectedComponents call.
            let mut visit = |nx: usize, ny: usize| {
                let ni = ny * w + nx;
                if seen.get(ni).copied() == Some(false) && probs.get(ni).is_some_and(|&p| p >= threshold) {
                    if let Some(s) = seen.get_mut(ni) {
                        *s = true;
                    }
                    stack.push(ni);
                }
            };
            if x > 0 {
                visit(x - 1, y);
            }
            if x + 1 < w {
                visit(x + 1, y);
            }
            if y > 0 {
                visit(x, y - 1);
            }
            if y + 1 < h {
                visit(x, y + 1);
            }
        }
        let mean = if count == 0 { f32::NAN } else { (sum / count as f64) as f32 };
        out.push(Component { rect: [l as u32, t as u32, r as u32, b as u32], mean });
    }
    out
}

/// Expand a box uniformly by the Paddle `unclip` distance, `area * ratio / perimeter`. A box
/// with no area or perimeter is returned unchanged.
pub(crate) fn unclip(rect: [f32; 4], ratio: f32) -> [f32; 4] {
    let (w, h) = ((rect[2] - rect[0]).max(0.0), (rect[3] - rect[1]).max(0.0));
    let (area, perimeter) = (w * h, 2.0 * (w + h));
    if !area.is_finite() || !perimeter.is_finite() || perimeter <= 0.0 || !ratio.is_finite() {
        return rect;
    }
    let d = area * ratio / perimeter;
    [rect[0] - d, rect[1] - d, rect[2] + d, rect[3] + d]
}

/// Clamp a box to `0..width` × `0..height`; `None` when it is not finite or has no area.
fn clamp_box(rect: [f32; 4], width: u32, height: u32) -> Option<[f32; 4]> {
    if rect.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (w, h) = (width as f32, height as f32);
    let out = [rect[0].clamp(0.0, w), rect[1].clamp(0.0, h), rect[2].clamp(0.0, w), rect[3].clamp(0.0, h)];
    (out[2] > out[0] && out[3] > out[1]).then_some(out)
}

/// Crop the RGBA `rect` of `image` and return it as interleaved RGB with its integer size.
/// `None` for a box that rounds to nothing.
fn crop_rgb(image: &OcrImage, rect: [f32; 4]) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (image.width(), image.height());
    let x0 = rect[0].floor().clamp(0.0, w as f32) as u32;
    let y0 = rect[1].floor().clamp(0.0, h as f32) as u32;
    let x1 = rect[2].ceil().clamp(0.0, w as f32) as u32;
    let y1 = rect[3].ceil().clamp(0.0, h as f32) as u32;
    let (cw, ch) = (x1.checked_sub(x0)?, y1.checked_sub(y0)?);
    if cw == 0 || ch == 0 {
        return None;
    }
    let stride = w as usize * 4;
    let mut out = Vec::with_capacity(cw as usize * ch as usize * 3);
    for y in y0..y1 {
        for x in x0..x1 {
            let o = y as usize * stride + x as usize * 4;
            let px = image.rgba().get(o..o + 4)?;
            out.extend_from_slice(&px[..3]);
        }
    }
    Some((cw, ch, out))
}

/// The text of one class index under the three dictionary layouts Paddle exports use:
/// blank + `dict`, blank + `dict` + a trailing space, or (some converters) `dict` with no
/// blank. `None` for the CTC blank (class 0) and for an out-of-range class.
fn class_text(index: usize, classes: usize, dict: &[String]) -> Option<&str> {
    if index == 0 {
        return None;
    }
    if classes == dict.len() {
        return dict.get(index).map(String::as_str);
    }
    if classes == dict.len() + 2 && index + 1 == classes {
        return Some(" ");
    }
    dict.get(index - 1).map(String::as_str)
}

/// The winner's probability within one time step's class distribution, given its logit or
/// probability `best_value`.
///
/// PP-OCR's recognition export already ends in a softmax: every row is a distribution (all
/// entries in 0–1, summing to 1), so its values *are* the probabilities and are read as such.
/// A model that stops at the logits gets the softmax applied here, with the winner's value
/// subtracted so a large logit cannot overflow. Non-finite entries carry no probability and are
/// left out of the normaliser. `None` when the row has no scoreable value at all.
fn winner_probability(row: &[f32], best_value: f32) -> Option<f64> {
    if !best_value.is_finite() {
        return None;
    }
    let distribution =
        row.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) && (row.iter().map(|&v| f64::from(v)).sum::<f64>() - 1.0).abs() <= 0.01;
    if distribution {
        // A softmax over an already-normalised row would flatten every confidence towards
        // 1 / classes (about 0.002 here), hiding the very uncertainty the score is for.
        return Some(f64::from(best_value).clamp(0.0, 1.0));
    }
    let denominator: f64 = row.iter().filter(|v| v.is_finite()).map(|&v| f64::from(v - best_value).exp()).sum();
    (denominator.is_finite() && denominator > 0.0).then_some(1.0 / denominator)
}

/// Greedy CTC decode and the confidence of what it read, on the 0–1 scale the caller rescales
/// to [`Word::with_confidence`]'s 0–100: the mean probability of the classes the decoder
/// actually emitted, over the steps that emitted one (a repeat collapsed into the previous
/// class and the CTC blank contribute no text, so they are not scored). When nothing is
/// emitted — an empty or whitespace-only read — the mean of every step's winning probability
/// stands in, which is what the model thought of the best class it had. `NaN` when the buffer
/// holds no scoreable step at all, so the word carries no confidence rather than a made-up one.
///
/// The decode itself: argmax per time step, collapse repeats, drop class 0, map the rest
/// through [`class_text`]. A short or inconsistent logits buffer decodes what it holds.
pub(crate) fn ctc_decode_with_score(logits: &[f32], steps: usize, classes: usize, dict: &[String]) -> (String, f32) {
    if classes == 0 {
        return (String::new(), f32::NAN);
    }
    let mut out = String::new();
    let mut previous = usize::MAX;
    // The emitted classes' probabilities, and every step's winner for the empty-read fallback.
    let (mut emitted, mut emitted_steps) = (0.0f64, 0usize);
    let (mut winners, mut winner_steps) = (0.0f64, 0usize);
    for t in 0..steps {
        let Some(start) = t.checked_mul(classes) else { break };
        let Some(end) = start.checked_add(classes) else { break };
        let Some(row) = logits.get(start..end) else { break };
        let mut best = 0usize;
        let mut best_value = f32::NEG_INFINITY;
        for (i, &v) in row.iter().enumerate() {
            if v > best_value {
                best = i;
                best_value = v;
            }
        }
        // The winner's probability, from a row that is already a distribution or from logits.
        let probability = winner_probability(row, best_value);
        if let Some(p) = probability {
            winners += p;
            winner_steps += 1;
        }
        if best != previous
            && let Some(text) = class_text(best, classes, dict)
        {
            out.push_str(text);
            if let Some(p) = probability {
                emitted += p;
                emitted_steps += 1;
            }
        }
        previous = best;
    }
    let score = if emitted_steps > 0 {
        emitted / emitted_steps as f64
    } else if winner_steps > 0 {
        winners / winner_steps as f64
    } else {
        f64::NAN
    };
    (out, score as f32)
}

/// Group words into lines: sort by top then left, and join a word to the closest existing
/// line whose vertical centre is within half the taller height. Words within a line are
/// ordered left to right; lines keep top-to-bottom order.
pub(crate) fn group_lines(words: Vec<Word>) -> Vec<Line> {
    let mut words: Vec<Word> = words.into_iter().filter(|w| !w.text.is_empty() && w.rect.iter().all(|v| v.is_finite())).collect();
    words.sort_by(|a, b| a.rect[1].total_cmp(&b.rect[1]).then_with(|| a.rect[0].total_cmp(&b.rect[0])));

    struct Open {
        words: Vec<Word>,
        top: f32,
        bottom: f32,
    }
    let mut lines: Vec<Open> = Vec::new();
    for word in words {
        let (top, bottom) = (word.rect[1], word.rect[3]);
        let centre = (top + bottom) / 2.0;
        let height = (bottom - top).max(0.0);
        let target = lines.iter().enumerate().rev().find(|(_, line)| {
            let line_centre = (line.top + line.bottom) / 2.0;
            let line_height = (line.bottom - line.top).max(0.0);
            (centre - line_centre).abs() <= 0.5 * height.max(line_height)
        });
        match target {
            Some((i, _)) => {
                if let Some(line) = lines.get_mut(i) {
                    line.top = line.top.min(top);
                    line.bottom = line.bottom.max(bottom);
                    line.words.push(word);
                }
            }
            None => lines.push(Open { words: vec![word], top, bottom }),
        }
    }
    lines
        .into_iter()
        .map(|mut line| {
            line.words.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
            Line { words: line.words }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_SIDE;

    fn dict(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_string()).collect()
    }

    /// Just the decoded text: the score has its own test, and this keeps the decode tests
    /// reading as before the score existed.
    fn decode(logits: &[f32], steps: usize, classes: usize, dict: &[String]) -> String {
        ctc_decode_with_score(logits, steps, classes, dict).0
    }

    #[test]
    fn detection_input_is_stride_aligned_and_capped() {
        assert_eq!(detect_size(1654, 2339), (672, 960));
        assert_eq!(detect_size(100, 50), (96, 64), "each side rounds to the nearest 32");
        assert_eq!(detect_size(1, 1), (32, 32), "a tiny page still gets the model's minimum");
        assert_eq!(detect_size(960, 960), (960, 960));
        assert_eq!(detect_size(1921, 40), (960, 32));
        // The caps in OcrImage guarantee the result is bounded.
        let (w, h) = detect_size(MAX_SIDE, MAX_SIDE);
        assert!(w.max(h) <= DETECT_MAX_SIDE as u32 && w % 32 == 0 && h % 32 == 0);
    }

    #[test]
    fn recognition_width_tracks_the_aspect_and_is_bounded() {
        assert_eq!(rec_width(480, 48), 480, "a long line keeps its own width, not a squash to 320");
        assert_eq!(rec_width(48, 48), 48);
        assert_eq!(rec_width(1, 1000), 1, "at least one column");
        assert_eq!(rec_width(0, 0), 1);
        assert_eq!(rec_width(100_000, 48), REC_HARD_MAX_WIDTH, "a hostile box is bounded");
    }

    #[test]
    fn bilinear_resize_interpolates_and_refuses_bad_input() {
        // A 2 × 2 grey ramp widened to 4 × 2: the middle columns interpolate.
        let src = [0u8, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255];
        let out = resize_bilinear(&src, 4, 2, 2, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 4);
        assert_eq!(&out[0..4], &[0, 0, 0, 255], "left edge is the source's left");
        assert_eq!(&out[12..16], &[255, 255, 255, 255], "right edge is the source's right");
        let middle = out[4];
        assert!(middle > 0 && middle < 255, "the in-between column interpolates, got {middle}");
        // Same size is an identity.
        assert_eq!(resize_bilinear(&src, 4, 2, 2, 2, 2), src);
        // Degenerate and inconsistent inputs are empty, not panics.
        assert!(resize_bilinear(&src, 4, 0, 2, 4, 2).is_empty());
        assert!(resize_bilinear(&src, 4, 2, 2, 0, 2).is_empty());
        assert!(resize_bilinear(&src, 0, 2, 2, 4, 2).is_empty());
        assert!(resize_bilinear(&src[..7], 4, 2, 2, 4, 2).is_empty());
        assert!(resize_bilinear(&src, 4, 2, 2, 0, 0).is_empty());
    }

    #[test]
    fn normalise_is_nchw_with_per_channel_statistics() {
        // One red pixel: r = 255, g = b = 0.
        let rgb = [255u8, 0, 0];
        let n = normalise(&rgb, 1, 1, [0.5; 3], [0.5; 3]);
        assert_eq!(n, vec![1.0, -1.0, -1.0], "NCHW planes, (255/255 - .5)/.5 and (0 - .5)/.5");
        // The planes are channel-major, not interleaved.
        let n = normalise(&[255, 0, 0, 0, 0, 0], 2, 1, [0.5; 3], [0.5; 3]);
        assert_eq!(n, vec![1.0, -1.0, -1.0, -1.0, -1.0, -1.0]);
        // ImageNet statistics put mid grey just below zero for every channel.
        let n = normalise(&[128, 128, 128], 1, 1, DETECT_MEAN, DETECT_STD);
        assert!(n.iter().all(|v| v.abs() < 0.5), "mid grey is nearest the ImageNet means: {n:?}");
        // Mismatched buffers, zero sides and zero std are empty.
        assert!(normalise(&rgb[..2], 1, 1, [0.5; 3], [0.5; 3]).is_empty());
        assert!(normalise(&rgb, 0, 1, [0.5; 3], [0.5; 3]).is_empty());
        assert!(normalise(&rgb, 1, 1, [0.5; 3], [0.0; 3]).is_empty());
        assert!(normalise(&rgb, 1, 1, [f32::NAN; 3], [0.5; 3]).is_empty());
    }

    #[test]
    fn connected_boxes_finds_regions_and_their_means() {
        // Two separate 2 × 2 blobs on a 6 × 4 map.
        let (w, h) = (6u32, 4u32);
        let mut probs = vec![0.0f32; (w * h) as usize];
        for y in 0..2 {
            for x in 0..2 {
                probs[(y * w + x) as usize] = 0.8;
            }
        }
        for y in 2..4 {
            for x in 4..6 {
                probs[(y * w + x) as usize] = 0.9;
            }
        }
        let found = connected_boxes(&probs, w, h, 0.5);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].rect, [0, 0, 2, 2]);
        assert!((found[0].mean - 0.8).abs() < 1e-6);
        assert_eq!(found[1].rect, [4, 2, 6, 4]);
        assert!((found[1].mean - 0.9).abs() < 1e-6);
        // Below the threshold there is nothing.
        assert!(connected_boxes(&probs, w, h, 0.95).is_empty());
        // A mismatched buffer yields nothing instead of panicking.
        assert!(connected_boxes(&probs[..15], w, h, 0.5).is_empty());
        assert!(connected_boxes(&probs, 0, h, 0.5).is_empty());
        // NaN probabilities are not text.
        let mut nan = probs.clone();
        nan[0] = f32::NAN;
        let found = connected_boxes(&nan, w, h, 0.5);
        assert_eq!(found.first().map(|c| c.rect), Some([0, 0, 2, 2]), "the NaN pixel is not text, the rest of the blob is");
    }

    #[test]
    fn unclip_expands_by_the_paddle_distance() {
        // 100 × 10: area 1000, perimeter 220, distance = 1000 × 1.5 / 220.
        let out = unclip([10.0, 20.0, 110.0, 30.0], 1.5);
        let d = 1000.0 * 1.5 / 220.0;
        assert!((out[0] - (10.0 - d)).abs() < 1e-4, "{out:?}");
        assert!((out[2] - (110.0 + d)).abs() < 1e-4, "{out:?}");
        assert!((out[1] - (20.0 - d)).abs() < 1e-4, "{out:?}");
        assert!((out[3] - (30.0 + d)).abs() < 1e-4, "{out:?}");
        // Degenerate boxes are unchanged.
        assert_eq!(unclip([5.0, 5.0, 5.0, 5.0], 1.5), [5.0, 5.0, 5.0, 5.0]);
        assert_eq!(unclip([0.0, 0.0, 1.0, 0.0], 1.5), [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn clamp_and_crop_stay_inside_the_image() {
        assert_eq!(clamp_box([-5.0, -5.0, 20.0, 20.0], 10, 10), Some([0.0, 0.0, 10.0, 10.0]));
        assert_eq!(clamp_box([5.0, 5.0, 5.0, 6.0], 10, 10), None);
        assert_eq!(clamp_box([f32::NAN, 0.0, 1.0, 1.0], 10, 10), None);
        let rgba: Vec<u8> = (0..(4 * 4 * 4)).map(|i| (i % 256) as u8).collect();
        let image = OcrImage::new(4, 4, rgba).unwrap();
        let (w, h, rgb) = crop_rgb(&image, [1.0, 1.0, 3.0, 3.0]).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(rgb.len(), 12);
        assert_eq!(&rgb[..3], &[20, 21, 22], "the crop starts at (1, 1) in the original");
        assert!(crop_rgb(&image, [9.0, 9.0, 10.0, 10.0]).is_none());
    }

    #[test]
    fn ctc_decode_collapses_repeats_and_skips_the_blank() {
        let alphabet = dict(&["a", "b"]);
        // Blank + dict: classes 3.
        let logits = [
            0.9, 0.0, 0.0, // blank
            0.0, 0.9, 0.0, // a
            0.0, 0.9, 0.0, // a, repeated: collapsed
            0.0, 0.0, 0.9, // b
            0.0, 0.0, 0.9, // b, repeated
            0.9, 0.0, 0.0, // blank
            0.0, 0.9, 0.0, // a
        ];
        assert_eq!(decode(&logits, 7, 3, &alphabet), "aba");
        // The same when the export appends a space class: classes = dict + 2.
        let logits = [0.0, 0.0, 0.0, 0.9, 0.9, 0.0, 0.0, 0.0];
        assert_eq!(decode(&logits, 2, 4, &alphabet), " ", "the last class is a space");
        // No blank class at all: class i maps to dict[i].
        let no_blank = [0.9, 0.0, 0.0, 0.9];
        assert_eq!(decode(&no_blank, 2, 2, &alphabet), "b", "class 0 is still the blank, class 1 maps to dict[1]");
        // A short buffer decodes only the complete steps it holds.
        assert_eq!(decode(&logits[..5], 2, 4, &alphabet), " ");
        assert_eq!(decode(&[], 1, 0, &alphabet), "");
        // All-NaN steps read as the blank, not as garbage text.
        assert_eq!(decode(&[f32::NAN, f32::NAN, f32::NAN], 1, 3, &alphabet), "");
    }

    /// The decoder's confidence: the mean softmax probability of the classes it emitted. A
    /// class that takes nearly all of its step reads near 1, a two-way tie near 0.5, and the
    /// empty-read fallback averages the steps' winners. The text never changes.
    #[test]
    fn ctc_confidence_is_the_mean_probability_of_the_emitted_classes() {
        let alphabet = dict(&["a", "b"]);
        // Blank + dict: three classes. Each winner holds e^10 against two logits of 0, and the
        // repeated `a` collapses into the previous step, so it is not scored twice.
        let confident = [
            10.0, 0.0, 0.0, // blank
            0.0, 10.0, 0.0, // a
            0.0, 10.0, 0.0, // a again: collapsed, not scored
            0.0, 0.0, 10.0, // b
        ];
        let (text, score) = ctc_decode_with_score(&confident, 4, 3, &alphabet);
        assert_eq!(text, "ab");
        assert_eq!(text, decode(&confident, 4, 3, &alphabet), "the confidence changes no text");
        assert!(score > 0.99, "a confident sequence scores near 1, got {score}");
        // A two-way tie between two text classes: the winner is a coin flip with its sibling.
        let (text, score) = ctc_decode_with_score(&[0.0, 10.0, 10.0], 1, 3, &alphabet);
        assert_eq!(text, "a", "the tie still goes to the first class");
        assert!((score - 0.5).abs() < 1e-3, "a two-way tie scores near 0.5, got {score}");
        // The real export ends in a softmax (each row sums to 1 over the 526 classes): those
        // rows are probabilities and are not softmaxed a second time, which would flatten every
        // confidence to 1/classes and hide the uncertainty the score exists for.
        let probabilities = [0.05, 0.9, 0.05, 0.05, 0.05, 0.9];
        let (text, score) = ctc_decode_with_score(&probabilities, 2, 3, &alphabet);
        assert_eq!(text, "ab");
        assert!((score - 0.9).abs() < 1e-6, "a distribution row scores its winner as-is, got {score}");
        // No class emitted (a blank-versus-class tie every step): the steps' mean winner stands.
        let (text, score) = ctc_decode_with_score(&[0.0, 0.0, 0.0, 0.0], 2, 2, &dict(&["a"]));
        assert_eq!(text, "");
        assert!((score - 0.5).abs() < 1e-6, "the fallback averages the winners, got {score}");
        // Nothing scoreable (no classes, empty buffer, all-NaN steps): no confidence at all.
        let (text, score) = ctc_decode_with_score(&[], 0, 0, &alphabet);
        assert_eq!(text, "");
        assert!(score.is_nan(), "an empty buffer carries no confidence, got {score}");
        let (text, score) = ctc_decode_with_score(&[f32::NAN, f32::NAN, f32::NAN], 1, 3, &alphabet);
        assert_eq!(text, "");
        assert!(score.is_nan(), "an all-NaN step carries no confidence, got {score}");
    }

    fn word(text: &str, l: f32, t: f32, r: f32, b: f32) -> Word {
        Word::new(text, [l, t, r, b], "paddle")
    }

    #[test]
    fn group_lines_joins_by_vertical_centre() {
        let words = vec![
            word("world", 60.0, 12.0, 110.0, 30.0),
            word("hello", 10.0, 10.0, 55.0, 30.0),
            word("next", 10.0, 60.0, 40.0, 78.0),
            word("line", 45.0, 62.0, 70.0, 80.0),
        ];
        let lines = group_lines(words);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["hello", "world"]);
        assert_eq!(lines[0].text(), "hello world");
        assert_eq!(lines[1].words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["next", "line"]);
        // Empty and non-finite words are dropped without panicking.
        let lines = group_lines(vec![word("", 0.0, 0.0, 1.0, 1.0), word("nan", f32::NAN, 0.0, 1.0, 1.0)]);
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// End to end on a real Thai page, when the models are installed.
    #[test]
    fn reads_the_thai_sample_page() {
        let Some(models) = PaddleModels::find() else {
            eprintln!("skipped: PaddleOCR models not installed");
            return;
        };
        let recognizer = PaddleRecognizer::load(&models).unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/thai_ocr/page-03.png");
        let (w, h, rgba) = read_png(&path).unwrap();
        let image = OcrImage::new(w, h, rgba).unwrap();
        let lines = recognizer.recognize(&image, &RecognizeOptions { language: "th".into() }).unwrap();
        let text: String = lines.iter().map(Line::text).collect::<Vec<_>>().join("\n");
        let words: Vec<&Word> = lines.iter().flat_map(|l| &l.words).collect();
        let confidences: Vec<f32> = words.iter().filter_map(|w| w.confidence).collect();
        eprintln!("{}", text.chars().take(80).collect::<String>());
        let (low, high) = confidences.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &c| (lo.min(c), hi.max(c)));
        let mean = confidences.iter().sum::<f32>() / confidences.len().max(1) as f32;
        eprintln!("paddle confidences on the sample: {low:.1}–{high:.1}, mean {mean:.1} over {} words", confidences.len());
        assert_eq!(confidences.len(), words.len(), "every word now carries a confidence, none are None");
        assert!(low >= 0.0 && high <= 100.0, "confidences stay on the 0–100 scale: {low}–{high}");
        assert!(mean > 50.0, "the sample reads well, so its mean confidence should be high, got {mean}");
        assert!(text.contains("ประชาธิปไตย"), "the sample should read ประชาธิปไตย, got: {text}");
        assert!(words.len() >= 20, "at least 20 words should be found, got {}", words.len());
    }

    /// Read a PNG into RGBA8. Test-only: the engine takes validated [`OcrImage`]s.
    fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buffer = vec![0; reader.output_buffer_size().ok_or("the PNG is too large")?];
        let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
        let data = buffer.get(..info.buffer_size()).ok_or("the PNG buffer is short")?;
        let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(data),
            png::ColorType::Rgb => {
                for px in data.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            png::ColorType::Grayscale => {
                for &v in data {
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for px in data.as_chunks::<2>().0 {
                    rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
            }
            png::ColorType::Indexed => return Err("indexed PNG did not expand".into()),
        }
        Ok((info.width, info.height, rgba))
    }
}
