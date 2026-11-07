//! Unicode search text has no visible glyphs. A generated Type 3 font supplies advance widths
//! and a ToUnicode map, with empty character programs; no display font is needed or embedded.
//! Adapted from the searchable-layer implementation in #433 (32e09fbe).

use std::collections::BTreeMap;

use pdfcraft_cos::{Dict, Document, Object, Stream};
use pdfcraft_edit::EditError;
use pdfcraft_ocr::PlacedWord;

/// Refuse oversized text or invalid geometry before cloning words or allocating fonts.
/// These limits apply to the complete input, including all editable export pages.
pub(crate) fn validate_words<'a>(words: impl IntoIterator<Item = &'a PlacedWord>) -> Result<usize, EditError> {
    let mut count = 0usize;
    let mut length = 0usize;
    for w in words {
        count += 1;
        length = length.checked_add(w.text.len()).ok_or_else(|| EditError::Invalid("OCR text layer is too large".into()))?;
        if count > 65536 || length > 4 * 1024 * 1024 {
            return Err(EditError::Invalid("OCR text layer is too large".into()));
        }
        if !w.origin.iter().chain(&w.across).chain(&w.up).all(|v| v.is_finite())
            || !w.across[0].hypot(w.across[1]).is_finite()
            || !w.up[0].hypot(w.up[1]).is_finite()
            || (0..2).any(|i| !(w.origin[i] + 0.2 * w.up[i]).is_finite() || !(w.origin[i] + w.across[i] + 0.2 * w.up[i]).is_finite())
        {
            return Err(EditError::Invalid("OCR text coordinates must be finite".into()));
        }
    }
    Ok(count)
}

pub(crate) fn add_text(doc: &mut Document, page: usize, words: &[PlacedWord]) -> Result<(), EditError> {
    validate_words(words)?;
    // Keep the existing Latin layout, including Helvetica's proportional advance widths.
    if words.iter().all(|w| {
        w.text
            .chars()
            .all(|c| matches!(c, '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' | '€' | '‚' | '„' | '…' | '‘' | '’' | '“' | '”' | '•' | '–' | '—' | '™'))
    }) {
        pdfcraft_edit::stamp(doc, page, "OCR", pdfcraft_ocr::text_layer(words, &pdfcraft_fonts::TextPlan::default()))?;
        return Ok(());
    }
    let p = pdfcraft_model::pages(doc).get(page).cloned().ok_or_else(|| EditError::Invalid("OCR page does not exist".into()))?;
    let mut res = p.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
    let mut fonts = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()).unwrap_or_default();
    let chars: Vec<char> = words.iter().flat_map(|w| w.text.chars()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    // Match the extractor's em box (-200..800). Baselines below are offset by 0.2 em,
    // so search/selection rectangles span the detected image box.
    let blank = doc.add(Object::Stream(Stream::from_raw(Dict::new(), b"1000 0 0 -200 1000 800 d1\n".to_vec())));
    let mut codes = BTreeMap::new();
    let mut names = Vec::new();
    for (index, chunk) in chars.chunks(255).enumerate() {
        let mut name = format!("PCOcr{index}");
        while fonts.contains(name.as_bytes()) {
            name.push('x');
        }
        let mut differences = vec![Object::Int(1)];
        let mut procs = Dict::new();
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (PdfCraft) /Ordering (Unicode) /Supplement 0 >> def\n/CMapName /PdfCraftOCR def\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
        );
        for (batch, part) in chunk.chunks(100).enumerate() {
            cmap.push_str(&format!("{} beginbfchar\n", part.len()));
            for (i, ch) in part.iter().enumerate() {
                let code = (batch * 100 + i + 1) as u8;
                let glyph = format!("g{code:02X}");
                differences.push(Object::name(&glyph));
                procs.set(glyph.into_bytes(), Object::Ref(blank));
                let mut units = [0u16; 2];
                let hex: String = ch.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
                cmap.push_str(&format!("<{code:02X}> <{hex}>\n"));
                codes.insert(*ch, (index, code));
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let map = doc.add(Object::Stream(Stream::flate(Dict::new(), cmap.as_bytes())));
        let mut encoding = Dict::new();
        encoding.set(b"Type".to_vec(), Object::name("Encoding"));
        encoding.set(b"Differences".to_vec(), Object::Array(differences));
        let mut font = Dict::new();
        font.set(b"Type".to_vec(), Object::name("Font"));
        font.set(b"Subtype".to_vec(), Object::name("Type3"));
        font.set(b"FontBBox".to_vec(), Object::Array(vec![0.into(), (-200).into(), 1000.into(), 800.into()]));
        font.set(b"FontMatrix".to_vec(), Object::Array(vec![Object::Real(0.001), 0.into(), 0.into(), Object::Real(0.001), 0.into(), 0.into()]));
        font.set(b"FirstChar".to_vec(), Object::Int(1));
        font.set(b"LastChar".to_vec(), Object::Int(chunk.len() as i64));
        font.set(b"Widths".to_vec(), Object::Array(vec![Object::Int(1000); chunk.len()]));
        font.set(b"Encoding".to_vec(), Object::Dict(encoding));
        font.set(b"CharProcs".to_vec(), Object::Dict(procs));
        font.set(b"Resources".to_vec(), Object::Dict(Dict::new()));
        font.set(b"ToUnicode".to_vec(), Object::Ref(map));
        fonts.set(name.as_bytes().to_vec(), Object::Ref(doc.add(Object::Dict(font))));
        names.push(name);
    }
    res.set(b"Font".to_vec(), Object::Dict(fonts));
    doc.update_dict(p.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(res)))?;
    // `3 Tr` and the font are graphics state, which `ET` doesn't reset: like
    // `pdfcraft_ocr::text_layer` (#3029), the text is drawn inside `q … Q` so the invisible
    // rendering mode cannot leak into later content streams on the page.
    let mut content = String::from("/OCR BMC\nq\nBT\n3 Tr\n");
    for w in words {
        let count = w.text.chars().count();
        if count == 0 || w.up[0].hypot(w.up[1]) <= 0.0 || w.across[0].hypot(w.across[1]) <= 0.0 {
            continue;
        }
        let mut start = 0;
        let mut run = Vec::new();
        let mut font_index = None;
        let emit = |content: &mut String, font: usize, start: usize, run: &[u8]| {
            let n = count as f64;
            let at = start as f64 / n;
            let hex: String = run.iter().map(|b| format!("{b:02X}")).collect();
            content.push_str(&format!(
                "/{} 1 Tf\n{} {} {} {} {} {} Tm <{hex}> Tj\n",
                names[font],
                w.across[0] / n,
                w.across[1] / n,
                w.up[0],
                w.up[1],
                w.origin[0] + at * w.across[0] + 0.2 * w.up[0],
                w.origin[1] + at * w.across[1] + 0.2 * w.up[1]
            ));
        };
        for (i, ch) in w.text.chars().enumerate() {
            let Some(&(font, code)) = codes.get(&ch) else { continue };
            if let Some(previous) = font_index
                && previous != font
            {
                emit(&mut content, previous, start, &run);
                run.clear();
                start = i;
            }
            font_index = Some(font);
            run.push(code);
        }
        if let Some(font) = font_index {
            emit(&mut content, font, start, &run);
        }
    }
    content.push_str("ET\nQ\nEMC\n");
    pdfcraft_edit::stamp(doc, page, "OCR", content.into_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};

    use super::*;

    /// A one-page document with an empty content stream, built the way `printcheck` does.
    fn page(doc: &mut Document) -> ObjRef {
        let pages = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|d| d.reference(b"Pages"))).unwrap();
        let mut p = Dict::new();
        p.set(b"Type".to_vec(), Object::name("Page"));
        p.set(b"Parent".to_vec(), Object::Ref(pages));
        p.set(b"MediaBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
        p.set(b"Contents".to_vec(), Object::Ref(doc.add(Object::Stream(Stream::from_raw(Dict::new(), Vec::new())))));
        let r = doc.add(Object::Dict(p));
        doc.update_dict(pages, |d| {
            let mut kids = d.get(b"Kids").and_then(|k| k.as_array().cloned()).unwrap_or_default();
            kids.push(Object::Ref(r));
            d.set(b"Count".to_vec(), Object::Int(kids.len() as i64));
            d.set(b"Kids".to_vec(), Object::Array(kids));
        })
        .unwrap();
        r
    }

    fn placed(text: &str) -> PlacedWord {
        PlacedWord { text: text.into(), origin: [72.0, 700.0], across: [60.0, 0.0], up: [0.0, 10.0], confidence: None, source: "test".into() }
    }

    /// The `PCOcr…` fonts this layer added to page 1, with their dictionaries.
    fn ocr_fonts(doc: &Document) -> Vec<(String, Dict)> {
        let page = pdfcraft_model::pages(doc).into_iter().next().expect("the test page");
        let res = doc.resolve(page.dict.get(b"Resources").expect("resources")).as_dict().cloned().expect("a resource dict");
        let fonts = doc.resolve(res.get(b"Font").expect("fonts")).as_dict().cloned().expect("a font dict");
        fonts
            .iter()
            .filter(|(name, _)| name.starts_with(&b"PCOcr"[..]))
            .map(|(name, font)| (String::from_utf8_lossy(name).into_owned(), doc.resolve(font).as_dict().cloned().expect("a font dict")))
            .collect()
    }

    /// The decoded page content (all its streams, in order).
    fn content(doc: &Document) -> String {
        let page = pdfcraft_model::pages(doc).into_iter().next().expect("the test page");
        let d = doc.get(page.obj).as_dict().cloned().unwrap_or_default();
        let mut bytes = Vec::new();
        let mut add = |o: &Object| {
            if let Object::Stream(s) = &*doc.resolve(o) {
                bytes.extend(s.decoded().unwrap_or_default());
            }
        };
        match d.get(b"Contents") {
            Some(Object::Array(a)) => a.iter().for_each(&mut add),
            Some(o) => add(o),
            None => {}
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Every `Tj` in the content: the font it names and the codes it shows, in order.
    fn shown(content: &str) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut font = String::new();
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix('/')
                && line.ends_with(" Tf")
            {
                font = rest.split_whitespace().next().unwrap_or_default().to_string();
            } else if line.ends_with(" Tj")
                && let (Some(start), Some(end)) = (line.find('<'), line.find('>'))
            {
                let hex = &line[start + 1..end];
                let codes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
                out.push((font.clone(), codes));
            }
        }
        out
    }

    /// The font's ToUnicode CMap, as code → UTF-16 code units.
    fn tounicode(doc: &Document, font: &Dict) -> BTreeMap<u8, Vec<u16>> {
        let Object::Stream(stream) = &*doc.resolve(font.get(b"ToUnicode").expect("a ToUnicode map")) else {
            panic!("ToUnicode is not a stream");
        };
        let cmap = String::from_utf8(stream.decoded().expect("a decodable CMap")).expect("a text CMap");
        let mut map = BTreeMap::new();
        let mut in_bfchar = false;
        for line in cmap.lines() {
            if line.ends_with("beginbfchar") {
                in_bfchar = true;
            } else if line == "endbfchar" {
                in_bfchar = false;
            } else if in_bfchar {
                let mut parts = line.split_whitespace();
                let (Some(src), Some(dst)) = (parts.next(), parts.next()) else { continue };
                let code = u8::from_str_radix(src.trim_matches(['<', '>']), 16).expect("a hex code");
                let hex = dst.trim_matches(['<', '>']);
                let units = (0..hex.len()).step_by(4).map(|i| u16::from_str_radix(&hex[i..i + 4], 16).expect("a UTF-16 unit")).collect();
                map.insert(code, units);
            }
        }
        map
    }

    /// Thai means Type 3, never the WinAnsi stamp: the page gains a `PCOcr` font with a
    /// `ToUnicode` stream.
    #[test]
    fn thai_text_gets_a_type3_font_with_a_tounicode_map() {
        let mut doc = Document::new_empty();
        page(&mut doc);
        add_text(&mut doc, 0, &[placed("น้ำใจ")]).expect("the layer is written");
        let fonts = ocr_fonts(&doc);
        assert_eq!(fonts.len(), 1, "{fonts:?}");
        let (name, font) = &fonts[0];
        assert!(name.starts_with("PCOcr"), "{name}");
        assert_eq!(font.get(b"Subtype").and_then(Object::as_name), Some(&b"Type3"[..]));
        assert!(font.get(b"ToUnicode").is_some_and(|o| matches!(&*doc.resolve(o), Object::Stream(_))));
        assert!(!tounicode(&doc, font).is_empty(), "the CMap maps the Thai codes");
    }

    /// Codes in the content map back, through the CMap, to the word's own UTF-16 units —
    /// including the combining mark in "น้ำใจ".
    #[test]
    fn the_cmap_maps_the_codes_back_to_the_words_utf16() {
        let word = "น้ำใจ";
        assert!(word.chars().any(|c| c == '\u{0e49}'), "the fixture carries a combining mark");
        let mut doc = Document::new_empty();
        page(&mut doc);
        add_text(&mut doc, 0, &[placed(word)]).expect("the layer is written");
        let (name, font) = &ocr_fonts(&doc)[0];
        let map = tounicode(&doc, font);
        let shown = shown(&content(&doc));
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(&shown[0].0, name);
        let units: Vec<u16> = shown[0].1.iter().flat_map(|code| map.get(code).cloned().unwrap_or_default()).collect();
        assert_eq!(String::from_utf16(&units).expect("valid UTF-16"), word);
        assert!(map.values().any(|units| units == &[0x0e49u16]), "the combining mark is mapped");
    }

    /// More than 255 distinct characters need a second `PCOcr` font; the per-font code space
    /// starts at 1 in every font, so content never shows code 0.
    #[test]
    fn more_than_255_distinct_characters_spread_over_several_fonts_without_code_zero() {
        let text: String = (0..300).map(|i| char::from_u32(0x1000 + i).expect("a BMP character")).collect();
        let mut doc = Document::new_empty();
        page(&mut doc);
        add_text(&mut doc, 0, &[placed(&text)]).expect("the layer is written");
        let fonts = ocr_fonts(&doc);
        assert_eq!(fonts.len(), 2, "300 characters need two fonts: {:?}", fonts.iter().map(|(n, _)| n).collect::<Vec<_>>());
        let lasts: Vec<i64> = fonts.iter().map(|(_, f)| f.get(b"LastChar").and_then(Object::as_int).unwrap_or_default()).collect();
        assert_eq!(lasts, vec![255, 45]);
        // Every font carries its own ToUnicode map.
        assert!(fonts.iter().all(|(_, f)| !tounicode(&doc, f).is_empty()));
        let shown = shown(&content(&doc));
        assert_eq!(shown.len(), 2, "the word continues in the second font");
        assert!(shown.iter().all(|(_, codes)| codes.iter().all(|c| *c != 0)), "code 0 is never emitted: {shown:?}");
        assert!(shown.iter().any(|(font, codes)| font == "PCOcr1" && codes.len() == 45), "{shown:?}");
    }

    /// A word with a zero-length box is skipped: nothing is shown for it, while a good word in
    /// the same layer still is.
    #[test]
    fn a_word_with_a_degenerate_box_is_skipped() {
        let mut across_zero = placed("ใจ");
        across_zero.across = [0.0, 0.0];
        let mut up_zero = placed("น้ำ");
        up_zero.up = [0.0, 0.0];
        let mut doc = Document::new_empty();
        page(&mut doc);
        add_text(&mut doc, 0, &[across_zero, up_zero, placed("น้ำใจ")]).expect("the layer is written");
        let shown = shown(&content(&doc));
        assert_eq!(shown.len(), 1, "only the word with a real box is shown: {shown:?}");
        assert_eq!(shown[0].1.len(), 5, "the shown word is the whole of \"น้ำใจ\"");
    }
}
