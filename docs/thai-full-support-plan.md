# แผนพัฒนา: Thai Full Support (ฟอนต์ + OCR)

> **สถานะล่าสุด (2026-11-07):** ดูหัวข้อ 0 ด้านล่างสำหรับงานที่ปิดแล้วและที่ยังติดข้อจำกัดภายนอก

## 0. สถานะความคืบหน้า

### ปิดแล้ว (มี commit ใน repo นี้)
| งาน | Commit |
|---|---|
| T1 เสนอภาษาไทย + route ไป Tesseract (ไม่ส่งให้ ocrs) | `376ecbe4` |
| T2 bundle `tha`/`eng` + SHA-256 + `TESSDATA_PREFIX` แบบมีเงื่อนไข | `f5d5ef92` |
| T4 อ่านไทยแบบ `tha+eng` (CER 6.2% → 1.5%) + TSV ผ่าน `-c tessedit_create_tsv=1` | `f5d5ef92` |
| T7 ต่อ cluster ไทยไม่ใส่ช่องว่าง + dictionary segmentation (ICU4X) | `fb9cdc6a` |
| T6 searchable text layer: round-trip/save/marks/Type3 tests | `46f9c025` |
| วรรณยุกต์ไทยใน editable output (ไม่เป็น `?`, วางตาม GPOS offset) | `f1e8fdf3` |
| Visual regression: ink ในกล่อง/ไม่ล้น/วรรณยุกต์เปลี่ยนภาพ | `9c5cebb0` |
| Audit ข้อ 3–4: WinAnsi gate + Helvetica บนหน้า mixed | `48f04bf0` |
| ตัดบรรทัดไทยตามขอบเขตคำ (Add Text) | `ecef5fc2` |
| TIS-620/CP874 ทั้ง extraction และ renderer (vendored patch 76) | `0d72ad49` |
| เปิด Thai UI: ตรวจพบว่า catalog มีแล้ว + เพิ่ม tests `thai_catalog_loads` และแก้ docs ที่ค้าง | `877e3a37` |
| D6 accuracy corpus (public domain): ASEAN Charter 6 หน้า 200 dpi, 4,199 ตัวอักษร, CER 2.12% | `cargo xtask ocr-bench` |

### ทำได้บางส่วน (partial)
| งาน | ทำได้แล้ว | เหลือ |
|---|---|---|
| Edit text ไทย (ข้อความเดิมในหน้า) | Type 3 fallback เลือก `document_thai_font()` สำหรับ U+0E00–U+0E7F (ฟอนต์ชื่อ `/PCThai`), bake GPOS offset ของ mark ลง CharProc พร้อม zero advance, build ที่ไม่มี Thai face แจ้งชัดแทนการล้มบน missing glyph; mark ไทยรวม cluster กับ base ตอน cursor/selection/delete (`is_thai_mark`, SARA AM ต่อ cluster ก่อนหน้า); clause ไทยที่แก้ตัดบรรทัดใหม่ตามขอบเขตคำจากพจนานุกรม | ใช้ฟอนต์ไทยที่หน้า PDF ฝังอยู่เอง (Slice A); styled-region path (mixed style) ยังตัดไทยที่ช่องว่างเท่านั้น |

### ยังไม่ปิด — ติดข้อจำกัดภายนอกหรือต้องคนไทยตรวจ
| งาน | เหตุที่ยังทำไม่ได้ |
|---|---|
| C1 catalog ภาษาไทย | **มีอยู่แล้ว** ใน craft-i18n ที่ rev ที่ pin ไว้ (3,300 entries, 0 ช่องว่าง) — เปิดใช้ + เพิ่ม tests/docs แล้ว (`877e3a37`); เหลือ native review เท่านั้น |
| A2 Thai Bold / A3 oblique | craft-fonts pin `b076417b` มีเฉพาะ Noto Sans Thai Regular |
| D3 offline ONNX Thai | ยังไม่มีโมเดลที่ license เปิดและรันบน rten ได้; ใช้ tesseract `tha` แทน |
| B3 justify / B5 PDF/UA tagging | ต่อยอดจาก segmentation ที่มีแล้ว แต่ยังไม่เริ่ม |
| วรรณยุกต์แทนที่ด้วย glyph อื่นจะผ่าน pixel test | ต้องมี golden image (ยืนยันรูปร่าง) ไม่ใช่แค่ pixel diff |


> เป้าหมาย: ให้ PdfCraft Thai รองรับภาษาไทยครบทั้ง **การแสดง/แก้ไข/บันทึกข้อความไทย** และ **OCR ไทย**
> โดยไม่ต้องพึ่งเครื่องมือภายนอกแบบบังคับ และไม่มีการกลายเป็น `?` ในทุกช่องทางที่ผู้ใช้พิมพ์ข้อความ

สถานะปัจจุบันอ้างอิงจากโค้ดใน repo นี้ (graphify graph 44,388 nodes) และ docs/localization*, docs/gaps #7,
docs/file-format-parity OCR table

---

## 1. สถานะปัจจุบัน (มีแล้ว vs ขาด)

### 1.1 ฟอนต์ / การเรียงข้อความไทย — มีแล้ว
| ความสามารถ | หลักฐานในโค้ด |
|---|---|
| Thai shaping ด้วย Harfrust (THAI script, MonotoneGraphemes) | `crates/fonts/src/shaping.rs:90` `shape_thai()` |
| ฟอนต์ Noto Sans Thai ผ่าน craft-fonts (ถ้า build ด้วย `CRAFT_FONTS_DIR`) | `crates/fonts/src/craft.rs:70` `ui_thai_fonts()` / `document_thai_font()` |
| UI font stack มี Thai face | `crates/ui-egui/src/theme.rs:392` |
| Web build ฝัง Thai face | `crates/fonts/build.rs:111` |
| Type 3 embedding + ToUnicode + ActualText | `crates/fonts/src/pdf.rs`, `crates/edit/src/added/draw.rs` |
| ตัดบรรทัดแบบ cluster-safe (Sara Am / วรรณยุกต์ไม่หลุด) | `crates/edit/src/added/layout.rs:78` `thai_lines()` |
| Add Text ผ่าน engine / CLI / MCP + test | `crates/automation/tests/it/automation.rs:9177` |
| OCR text layer รองรับอักษรนอก WinAnsi ผ่าน Type 3 | `crates/fonts/src/plan.rs` (`TextPlan`) |

### 1.2 ช่องว่างที่ยังทำให้ยังไม่ "full support"
| # | ช่องว่าง | หลักฐาน | ผลกระทบ |
|---|---|---|---|
| F1 | ฟอนต์ไทยเป็น optional (ไม่มี `CRAFT_FONTS_DIR` = ไม่มีฟอนต์) | `crates/fonts/src/craft.rs` คืน empty | build ท้องถิ่น/CI บางแบบพิมพ์ไทยไม่ได้ |
| F2 | มีเฉพาะ Regular — ไม่มี Bold/Italic | `document_thai_font()` คืน face แรกเท่านั้น | หัวข้อ/ตัวหนาในเอกสารไทยทำไม่ได้ |
| F3 | ไม่มี TIS-620 / CP874 (WinAnsi-Thai) | `crates/fonts/src/encodings.rs` มีแค่ WinAnsi/MacRoman/Standard | PDF ไทยเก่า ๆ (Legacy 1-byte) extract/ค้นหาเพี้ยน |
| F4 | Forms / Fill & Sign ไม่อยู่ใน shaped path — อักษรนอก WinAnsi กลายเป็น `?` | `crates/forms/src/appearance.rs:12` | กรอกฟอร์มไทยไม่เห็นข้อความ (มีแค่ NeedAppearances) |
| F5 | Edit text (ข้อความเดิมในหน้า) ไม่มีเส้นทาง Thai cluster | `crates/edit/src/added/*` เท่านั้นที่มี `is_thai` | แก้ข้อความไทยเดิมไม่ได้/เพี้ยน |
| F6 | ตัดบรรทัดไทยไม่ใช้พจนานุกรม (UAX #14 ต้องใช้ dictionary) | `crates/edit/src/linebreak.rs:9` "Thai, whose words need a dictionary" | ตัดกลางคำ, justify ไม่ได้, PDF/UA tag คำไม่ได้ |
| F7 | ไม่มี catalog ภาษาไทย (`th`) | `docs/localization.md` รายชื่อภาษาไม่มี ไทย | UI ยังเป็นอังกฤษ |
| F8 | ไม่มี Thai date/number/plural ในระบบ i18n | `craft-i18n` ไม่มี `locales/th/` | วันที่/ตัวเลขยังเป็นรูปแบบอังกฤษ |

### 1.3 OCR ไทย
| # | สถานะ | หลักฐาน |
|---|---|---|
| O1 | tesseract allow-list **ไม่มี `th`/`tha`** | `crates/ocr/src/tesseract.rs:32` `LANGUAGE_CODES` |
| O2 | ocrs (builtin) อ่านได้เฉพาะ Latin ไม่มีสระ/วรรณยุกต์ | `crates/ocr/src/ocrs.rs:30` `LANGUAGES = [("en","English")]`, `ALPHABET` |
| O3 | UI dropdown ใช้รายการรวมจาก engines → ไทยไม่มีให้เลือก | `crates/ui-egui/src/workflows/ocr_ui.rs:258` |
| O4 | โมเดล ocrs โหลดคู่ detection+recognition แบบตายตัว | `crates/ocr/src/ocrs.rs` `Models::in_dir` |
| O5 | ไม่มีพจนานุกรม/การแบ่งคำไทยสำหรับ review + searchable layer | `crates/engine/src/ocr/dictionary.rs` (plain text ต่อภาษา) |
| O6 | เอกสารยอมรับเองว่า "OCR reads only unaccented Latin" | `docs/gaps.md:174`, `docs/file-format-parity.md:104` |

---

## 2. นิยามของ "Full Support" (Definition of Done)

1. **Glyph coverage**: ข้อความไทยทุกช่วง U+0E00–U+0E7F (รวมสระบน/ล่างซ้อน 2 ชั้น, ฤๅ, ฦๅ, ตัวเลขไทย ๐–๙) แสดงผลได้โดยไม่เป็น `?` ในทุก surface: Viewer, Add text, Edit text, Forms, Comments, Signatures, OCR output, Search highlight
2. **Round-trip**: บันทึก → เปิดใหม่ → extract → search ได้ข้อความไทยตรงกับที่พิมพ์ (NFC, ไม่แตก Sara Am ซ้ำ, ลำดับวรรณยุกต์ถูก)
3. **Line breaking**: ตัดบรรทัดตามพจนานุกรมไทย (ไม่ตัดกลางคำ) ตรงกับ ICU reference ≥ 95% ของชุดทดสอบ
4. **Bold**: เอกสารไทยมี Regular + Bold (และ oblique สังเคราะห์ถ้าต้องการ italic)
5. **UI**: แปลไทยครบ 2,798 keys, วันที่/ตัวเลขแบบไทย, เลือกภาษาไทยได้จาก Auto-detect (`th`, `th_TH`)
6. **OCR**: อ่านไทยได้ offline หรือผ่าน tesseract ที่ระบุชัด, char accuracy ≥ 95% บน scan 300 dpi สะอาด, ≥ 90% บน 200 dpi, ค้นหาคำไทยเจอ
7. **Cross-platform**: macOS / Windows / Linux desktop + web (โดยเอกสารข้อจำกัด web OCR ให้ชัด)

---

## 3. แผนงานแบ่งเป็น 5 track

### Track A — ฟอนต์และการเรนเดอร์ไทย (≈ 110–170 h)

| ID | งาน | ไฟล์หลัก | ประมาณ |
|---|---|---|---|
| A1 | **บังคับมีฟอนต์ไทยในทุก build**: release/CI gate ตรวจว่ามี Thai face; เพิ่ม fallback ที่ bundled face (ถ้าไม่มี craft-fonts ให้ใช้ Noto Sans Thai ที่ฝังมากับแอป) | `crates/fonts/build.rs`, `craft.rs`, `xtask/src/gates.rs` | 6–10 h |
| A2 | **Thai Bold**: เพิ่ม Noto Sans Thai Bold เข้า craft-fonts pin, เปลี่ยน API เป็น `document_thai_font(bold: bool)` แบบเดียวกับ Hebrew, ผูกกับ style ของ Add text/ฟอร์ม | `crates/fonts/src/craft.rs`, `shaping.rs`, `crates/edit/src/added/*` | 12–18 h |
| A3 | **Italic/oblique**: ไทยไม่มี italic จริง → สังเคราะห์ shear แบบเดียวกับ fallback ของ signature (`FALLBACK_SLANT`) พร้อม flag ใน metadata | `crates/fonts/src/shaping.rs`, `pdf.rs` | 8–12 h |
| A4 | **Legacy Thai encodings**: เพิ่มตาราง TIS-620/CP874 + WinAnsi-Thai Differences, ใช้เป็น fallback เมื่อไม่มี ToUnicode ตอน extract/search | `crates/fonts/src/encodings.rs`, `pdf.rs` | 20–30 h |
| A5 | **Forms ไทย**: `appearance.rs` เลือก face ตาม script (เหมือน edit) แทนการทิ้งเป็น `?`; คงค่าเดิม + NeedAppearances; คำนวณ width จาก shaped advance | `crates/forms/src/appearance.rs`, `values.rs` | 24–40 h |
| A6 | **Edit text ไทย (ข้อความเดิม)**: แยก run ตาม cluster, รักษา mark order, เขียนกลับพร้อม ToUnicode; จำกัดเฉพาะ run ที่ฟอนต์เดิมมี glyph ไม่ครบเท่านั้น | `crates/edit/src/*` (bidi/linebreak/added) | 30–50 h |
| A7 | **Viewer fallback ไทย**: ถ้า PDF ไม่ฝังฟอนต์และไม่มี order ที่รู้จัก → ใช้ Thai face ใน fallback chain พร้อมรายงาน substitution | `crates/fonts/src/fallback.rs`, `crates/render/src/text/*` | 10–16 h |
| A8 | **Metrics/ระยะบรรทัด**: กันวรรณยุกต์ซ้อนทับบรรทัดบน, ascent/descent ของ Noto Sans Thai ใน Type 3 | `crates/fonts/src/pdf.rs`, `crates/render/src/text/layout.rs` | 8–12 h |

### Track B — Thai text layout & segmentation (≈ 60–95 h)

| ID | งาน | ไฟล์หลัก | ประมาณ |
|---|---|---|---|
| B1 | **Word segmentation ไทย**: เพิ่ม `icu_segmenter` (dictionary Thai/Lao/Khmer) เป็น optional feature, ทำ wrapper `segment_thai()` ใช้ร่วมกันทุกที่ | crate ใหม่ `crates/textlayout` หรือ `crates/fonts/src/segment.rs` | 24–36 h |
| B2 | **Line breaking ตาม UAX #14 + dictionary**: ต่อเข้ากับ `linebreak.rs` แทนการตัดที่ช่องว่าง; emergency break ที่ cluster boundary; เก็บ kinsoku rules เดิม | `crates/edit/src/linebreak.rs`, `added/layout.rs` | 14–22 h |
| B3 | **Justify ไทย**: กระจายช่องว่างระหว่าง "คำ" ตาม segmentation (ไม่ยืดตัวอักษร) | `crates/render/src/text/layout.rs` | 8–12 h |
| B4 | **Search/Find ไทย**: ค้นหาแบบ substring บน logical text, highlight ระดับ cluster, รองรับ mark ที่ซ้อน | `crates/render/src/text/search.rs`, `crates/ui-egui/src/*find*` | 8–12 h |
| B5 | **PDF/UA tagging ไทย**: ใช้ segmentation สร้าง struct element ระดับคำ | `crates/a11y/*` | 8–12 h |

### Track C — Thai UI localization (≈ 80–130 h)

| ID | งาน | ไฟล์หลัก | ประมาณ |
|---|---|---|---|
| C1 | **catalog `th`**: แปล `common.tsv` + `pdfcraft.tsv` ~2,800 entries (ใช้ศัพท์คอมพิวเตอร์ไทยมาตรฐาน) | craft-i18n (`locales/th/`), bump `craft-i18n.rev` | 50–80 h |
| C2 | **ลงทะเบียนภาษา**: เพิ่ม `th` ในชุดภาษาที่รองรับ + locale detect (`th`, `th_TH`, `th-TH`) + ชื่อใน Preferences | `crates/ui-egui/src/i18n.rs`, `app/prefs.rs` | 6–10 h |
| C3 | **รูปแบบไทย**: วันที่ พ.ศ./ค.ศ. (เลือกได้), ตัวเลขไทย ๐–๙ (ถ้าเปิด), plural rule CLDR `th` (one/other) | craft-i18n, `crates/ui-egui/src/*` | 10–16 h |
| C4 | **Font coverage test**: Thai UI string ทุกตัวอักษรมี glyph ใน face ที่ฝัง | `tests/it/fonts.rs` | 4–6 h |
| C5 | **Coverage gate**: `cargo xtask i18n-report` ผ่านเกณฑ์ไทย ≥ 98% | `xtask/src/*` | 6–8 h |

### Track D — Thai OCR (≈ 120–230 h)

| ID | งาน | ไฟล์หลัก | ประมาณ |
|---|---|---|---|
| D1 | **Quick win: tesseract Thai** — เพิ่ม `("th","tha")` ใน allow-list + ชื่อ + UI + docs + test mapping | `crates/ocr/src/tesseract.rs` | 3–5 h |
| D2 | **จัดหา/ติดตั้ง `tha.traineddata`**: `cargo xtask models` staging + SHA-256 + attribution + คู่มือ offline install | `xtask/src/models.rs`, `ATTRIBUTION.toml` | 8–12 h |
| D3 | **OCR ไทยแบบ offline (ocrs)**: ประเมิน ONNX Thai rec model (PaddleOCR `th`/multilingual, Apache-2.0) กับ `rten`; ทำ model registry ต่อภาษา (detection shared + recognition/alphabet per language) | `crates/ocr/src/ocrs.rs`, `crates/ocr/src/lib.rs` | 40–80 h |
| D4 | **Text layer ไทย**: word box ระดับ cluster, ToUnicode + ActualText, searchable layer ทำงานได้, editable output ผ่าน Type 3 Thai face (มี `TextPlan` อยู่แล้ว) | `crates/engine/src/ocr/layer.rs`, `crates/fonts/src/plan.rs` | 20–32 h |
| D5 | **Review/dictionary ไทย**: พจนานุกรมผู้ใช้แบบคำ (ต้องมี segmentation), grouping เป็น phrase, Re-OCR region | `crates/engine/src/ocr/dictionary.rs`, `crates/ui-egui/src/workflows/ocr_ui.rs` | 16–24 h |
| D6 | **Accuracy tuning**: benchmark 100–300 หน้า (300/200/150 dpi, เอียง, ถ่ายเอกสาร), ปรับ preprocessing | `crates/ocr/src/preprocess.rs`, `tests/` | 24–40 h |
| D7 | **Web build**: tesseract ถูก compile out บน wasm — ตัดสินใจ: ปิด OCR ไทยบน web + แจ้งผู้ใช้ หรือเปิดทาง ONNX | `crates/ocr/src/*`, docs | 6–10 h |
| D8 | **ROVER/auto**: เมื่อ ocrs ไทยอ่านไม่ได้ ให้ Automatic เลือก tesseract อัตโนมัติ (evidenced fallback) | `crates/ocr/src/merge.rs` | 8–12 h |

### Track E — QA / Release (≈ 50–80 h)

| ID | งาน | ประมาณ |
|---|---|---|
| E1 | Thai corpus: PDF ตัวอย่าง 30–50 ไฟล์ (Legacy TIS-620, Noto embedded, ฟอร์ม, scan) + golden outputs | 16–24 h |
| E2 | E2E tests: add/edit/forms/OCR/save/reopen/extract/search ครบทุก surface | 12–20 h |
| E3 | Performance: cache shaper ต่อ face, วัดต้นทุน segmentation ในหน้าขนาดใหญ่ | 8–12 h |
| E4 | Docs: ROADMAP, `docs/localization-parity.md`, `docs/file-format-parity.md` OCR table, README | 8–12 h |
| E5 | Licensing: Noto (OFL), tesseract (Apache-2.0), models (ตรวจ CC-BY-SA/Apache) + ATTRIBUTION.toml | 4–8 h |

**รวมประมาณการ: 420–705 ชั่วโมง** (ขนานได้ ~50–60% ด้วย 3–5 agents → ~180–300 h wall clock)

---

## 4. ลำดับส่งมอบ (Milestones)

### M1 — Quick Wins (1–2 สัปดาห์, ~25–40 h)
- D1 tesseract `tha` + UI + docs
- A2 Thai Bold (Regular+Bold)
- A1 release gate ตรวจ Thai face
- B4 search ไทยเบื้องต้น + E1 corpus เริ่มต้น
- อัปเดต docs/gaps + parity table

**เกต:** พิมพ์/บันทึก/เปิด/search ข้อความไทยใน Add text และ OCR ผ่าน tesseract ได้, ไม่มี `?`

### M2 — Layout & Editing (3–5 สัปดาห์, ~80–130 h)
- B1 + B2 segmentation/line-break (ICU4X)
- A5 Forms ไทย
- A6 Edit text ไทย
- A3 oblique, A7 viewer fallback
- B3 justify

**เกต:** ฟอร์มไทย, แก้ข้อความไทยเดิม, ตัดบรรทัดไม่กลางคำ, Bold ทำงาน

### M3 — Localization & OCR Quality (6–10 สัปดาห์, ~120–200 h)
- C1–C3 catalog ไทย + วันที่/ตัวเลข
- D3 offline Thai OCR (ถ้าประเมินแล้ว feasible) หรือยืนยัน tesseract-only
- D4/D5/D6 text layer + review + accuracy
- A4 legacy encodings

**เกต:** UI ไทยครบ, OCR แม่นตามเกณฑ์, PDF ไทยเก่า extract ได้

### M4 — Hardening (2–3 สัปดาห์, ~40–70 h)
- E2–E5, performance, web, licensing
- ปิด gaps #7 และ `ocr.ocr-languages` partial

---

## 5. ความเสี่ยงและการลดความเสี่ยง

| ความเสี่ยง | ผล | มาตรการ |
|---|---|---|
| ไม่มี ONNX Thai model ที่ license เปิดและรันบน rten ได้ | D3 ยืดเยื้อ | ทำ D1/D2 ให้เสร็จก่อน; ถ้าไม่ผ่าน ใช้ tesseract เป็นเส้นทางไทยทางการ + ระบุใน docs |
| พจนานุกรม ICU ทำให้ binary โต | +1–2 MB | feature flag เปิดเฉพาะภาษาไทย/ลาว; วัดขนาดก่อน merge |
| Type 3 ที่มี mark ซ้อนทำให้ไฟล์ใหญ่ | ไฟล์บวม | จำกัด glyph ต่อ font (มี `MAX_TYPE3_GLYPHS=240`), subset ต่อหน้า |
| tesseract บน Windows ต้องติดตั้งแยก | ผู้ใช้ตั้งไม่เป็น | installer แนะนำ + ตรวจ `--list-langs` + ปุ่มติดตั้ง/ลิงก์ |
| รูปแบบวันที่ พ.ศ. ขัดกับผู้ใช้บางกลุ่ม | confusion | ตั้งค่าเลือก พ.ศ./ค.ศ. (default ตาม locale) |
| Legacy TIS-620 ไม่มี ToUnicode | extract ผิด | A4 + corpus E1 เป็นตัวจับ |

---

## 6. ตัวชี้วัดความสำเร็จ (metrics)

- Thai glyph miss rate = 0 ในทุก UI string และทุกเอกสารทดสอบ
- Round-trip diff = 0 ตัวอักษร (normalize NFC แล้ว)
- Line-break agreement กับ ICU ≥ 95%
- OCR CER ≤ 5% (300 dpi), ≤ 10% (200 dpi)
- Thai catalog coverage ≥ 98% ของ 2,798 keys
- ไม่มี regression ใน Arabic/Hebrew/Devanagari (shape path เดียวกัน)

---

## 7. งานที่ทำได้ทันทีใน repo นี้

1. `crates/ocr/src/tesseract.rs` — เพิ่ม `("th","tha")` + `LANGUAGE_NAMES`
2. `crates/fonts/src/craft.rs` — เปลี่ยน `document_thai_font()` → `document_thai_font(bold)` และเพิ่ม face Bold ใน craft-fonts pin
3. `xtask/src/gates.rs` — gate ว่า release build มี Thai face
4. `tests/it/fonts.rs` — Thai coverage test (U+0E00–U+0E7F)
5. `docs/gaps.md` + `docs/file-format-parity.md` — อัปเดตสถานะเมื่อแต่ละข้อปิด
