//! The editable OCR output for Thai: the recognised word is drawn as visible text inside its
//! box, and its combining mark is really drawn. Synthetic image only.

use pdfcraft_engine::ocr::{OcrPage, PlacedWord};
use pdfcraft_engine::{Session, export};
use pdfcraft_render::{PageRenderer, RenderRequest};

const PAGE: (u32, u32) = (240, 120);
/// The recognised word's box on the source page: x, baseline y, width, height (user space).
const WORD: [f64; 4] = [40.0, 40.0, 80.0, 40.0];

/// A blank white 240 × 120 pt "scan"; the editable output only needs its page size.
fn blank_scan() -> Vec<u8> {
    let rgba = vec![255u8; PAGE.0 as usize * PAGE.1 as usize * 4];
    export::encode_png(PAGE.0, PAGE.1, &rgba).unwrap()
}

/// The editable-text output for one recognised word, opened as a document of its own.
fn editable(text: &str) -> (Session, pdfcraft_engine::DocId) {
    let mut source = Session::new();
    let scan = source.create_from_images(&[("scan.png".into(), blank_scan())]).unwrap();
    let id = source.open("scan.pdf", None, scan, None).unwrap();
    let found = OcrPage {
        page: 0,
        words: vec![PlacedWord {
            text: text.into(),
            origin: [WORD[0], WORD[1]],
            across: [WORD[2], 0.0],
            up: [0.0, WORD[3]],
            confidence: None,
            source: "ocrs".into(),
        }],
        already_on_page: 0,
        skipped: None,
        notes: Vec::new(),
    };
    let bytes = source.editable_ocr_bytes(id, &[found]).unwrap();
    let mut out = Session::new();
    let id = out.open("editable.pdf", None, bytes, None).unwrap();
    (out, id)
}

fn render(s: &Session, id: pdfcraft_engine::DocId) -> (u32, u32, Vec<u8>) {
    let doc = s.get(id).unwrap();
    let out = PageRenderer::new(doc.bytes.clone(), Default::default()).render(RenderRequest { page: 0, scale: 1.0, ..Default::default() });
    assert!(out.error.is_none(), "{:?}", out.error);
    (out.width, out.height, out.rgba.to_vec())
}

fn dark(rgba: &[u8], width: u32, x: u32, y: u32) -> bool {
    let i = ((y * width + x) * 4) as usize;
    u32::from(rgba[i]) + u32::from(rgba[i + 1]) + u32::from(rgba[i + 2]) < 384
}

/// The word's box in device pixels, y down, from the block the output actually drew.
fn box_px(s: &Session, id: pdfcraft_engine::DocId, height: u32) -> [u32; 4] {
    let blocks = s.get(id).unwrap().text_blocks(0);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let r = blocks[0].rect;
    let clamp = |v: f64| v.clamp(0.0, 1.0e9) as u32;
    [clamp(r[0]), clamp(f64::from(height) - r[3]), clamp(r[2]), clamp(f64::from(height) - r[1])]
}

/// The mark "น้ำ" carries is drawn: the output has ink in its box, none in a ring around it,
/// and removing the mark changes the render.
#[test]
fn editable_thai_text_renders_inside_its_box_with_its_mark() {
    let missing = pdfcraft_fonts::TextPlan::for_words(["น้ำ"]).missing();
    if missing > 0 {
        eprintln!("skipped: this build has no Thai craft-fonts face ({missing} characters missing)");
        return;
    }

    let (s, id) = editable("น้ำ");
    let (w, h, rgba) = render(&s, id);
    let [x0, y0, x1, y1] = box_px(&s, id, h);
    assert!(x1 > x0 + 10 && y1 > y0 + 10, "the word box is drawn: {x0},{y0}..{x1},{y1}");
    let inside: Vec<(u32, u32)> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).collect();
    let ink = inside.iter().filter(|(x, y)| dark(&rgba, w, *x, *y)).count();
    assert!(ink * 50 >= inside.len(), "the word is drawn in its box: {ink} of {} dark pixels", inside.len());

    // A ring 8 pt wide around the box, still on the page, keeps the paper clean.
    let pad = 8u32;
    let (ax, ay, bx, by) = (x0.saturating_sub(pad), y0.saturating_sub(pad), (x1 + pad).min(w), (y1 + pad).min(h));
    let mut ring_total = 0usize;
    let mut ring_ink = 0usize;
    for y in ay..by {
        for x in ax..bx {
            if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                continue;
            }
            ring_total += 1;
            if dark(&rgba, w, x, y) {
                ring_ink += 1;
            }
        }
    }
    assert!(ring_ink * 100 <= ring_total, "the ink stays in the box: {ring_ink} of {ring_total} ring pixels");

    // The control word drops the tone mark (น้ำ → นำ). If the mark were not drawn the two
    // renders would match; they must not.
    let (cs, cid) = editable("นำ");
    let (cw, ch, control) = render(&cs, cid);
    assert_eq!((cw, ch), (w, h));
    let mut differing = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let a = ((y * w + x) * 4) as usize;
            let b = ((y * cw + x) * 4) as usize;
            if rgba[a..a + 3].iter().zip(&control[b..b + 3]).any(|(p, q)| p.abs_diff(*q) > 24) {
                differing += 1;
            }
        }
    }
    assert!(differing * 100 >= inside.len(), "the tone mark is drawn: {differing} of {} pixels differ from the unmarked word", inside.len());
}
