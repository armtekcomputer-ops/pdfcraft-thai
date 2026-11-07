# แผนลด token + ใช้ Tesseract-Thai (mrolarik)

> เอกสารประกอบของ [thai-full-support-plan.md](thai-full-support-plan.md)
> เป้าหมาย: ลดต้นทุน token ของงาน Thai Full Support ลง ~50% และกำหนดวิธีใช้
> `https://github.com/mrolarik/Tesseract-Thai` อย่างถูกต้องตามสัญญาอนุญาต

---

## 1. ตรวจสอบ repo `mrolarik/Tesseract-Thai` (ผลจริง)

| หัวข้อ | ผล |
|---|---|
| เนื้อหา | README + 4 Jupyter notebooks + โฟลเดอร์ `data-text-img/` (14 รูปทดสอบ) |
| **โมเดล `tha.traineddata`** | **ไม่มี** — repo นี้ไม่ใช่ model repo |
| **LICENSE** | **ไม่มี** (GitHub license API ตอบ 404) → **ห้าม copy/redistribute** โค้ดหรือรูป |
| วิธีที่ repo สอน | `sudo apt-get install tesseract-ocr-tha` + `pytesseract.image_to_string()` |
| Tesseract ที่ใช้ | เวอร์ชันเก่า (~3.x/4.x, ปี 2018) — notebook ล้าสมัย |
| คุณค่าแท้จริง | (1) ยืนยันชื่อ language pack = **`tha`** (2) ตัวอย่าง Much/รูปทดสอบ (3) แนวทางประเมินผล |

**สรุป:** ใช้เป็น **เอกสารอ้างอิง/ระเบียบวิธี** เท่านั้น — ดึงโมเดลจริงจาก official tessdata

| แหล่งโมเดลที่ถูกต้อง | ขนาด `tha.traineddata` | License | ใช้เมื่อ |
|---|---|---|---|
| `tesseract-ocr/tessdata_fast` | **1.07 MB** | Apache-2.0 | ค่าเริ่มต้น (ฝังไปกับแอปได้) |
| `tesseract-ocr/tessdata_best` | 7.6 MB | Apache-2.0 | โหมดคุณภาพสูง (ดาวน์โหลดเพิ่ม) |
| `tesseract-ocr/tessdata` (standard) | ~2-3 MB | Apache-2.0 | ทางเลือกกลาง |

> หมายเหตุ: `tha.traineddata` จาก tessdata ทุกตัวสร้างจากชุดข้อมูลเดียวกับที่ mrolarik ใช้สอน
> แต่มีสัญญาอนุญาตชัดเจน + มี checksum ให้ตรวจ

---

## 2. การ integrate Tesseract-Thai เข้า PdfCraft

### 2.1 สถาปัตยกรรมที่มีอยู่ (ไม่ต้องสร้างใหม่)
- `crates/ocr/src/tesseract.rs` — เรียก `tesseract <png> stdout -l <lang> tsv` แบบ external process
  + allow-list ภาษา + timeout + output cap + temp dir ส่วนตัว
- `crates/ocr/src/merge.rs` — ROVER ระหว่าง ocrs กับ tesseract
- `crates/engine/src/ocr/layer.rs` — สร้าง searchable text layer
- `crates/ui-egui/src/workflows/ocr_ui.rs` — dropdown ภาษาจาก `Recognizer::languages()`

### 2.2 งานที่ต้องทำ (Track D ฉบับแก้ไข — ไม่ต้องทำ ONNX ไทยแล้ว)

| ID | งาน | ไฟล์ | ชั่วโมง |
|---|---|---|---|
| **T1** | เพิ่ม `("th","tha")` ใน `LANGUAGE_CODES` + `("th","Thai")` ใน `LANGUAGE_NAMES` | `crates/ocr/src/tesseract.rs:32` | **1–2 h** |
| **T2** | `cargo xtask models` ดาวน์โหลด `tha.traineddata` (fast) + SHA-256 pin + วางใน `models/tessdata/` | `xtask/src/models.rs` | 6–10 h |
| **T3** | ค้นหา language pack แบบ offline: `$PDFCRAFT_TESSDATA`, `models/tessdata/`, ระบบ (`--list-langs` เดิม) + รายงานว่ามี/ไม่มี | `crates/ocr/src/tesseract.rs` | 6–10 h |
| **T4** | ค่าเริ่มต้น Thai: psm 6/4, `-l tha`, user-words/dictionary path (tesseract `user_words`) | `crates/ocr/src/tesseract.rs` | 4–8 h |
| **T5** | UI: แสดง "ไทย" ใน dropdown, เตือนถ้าไม่มี `tha`, ปุ่มดาวน์โหลด/ลิงก์คู่มือ | `crates/ui-egui/src/workflows/ocr_ui.rs` | 4–8 h |
| **T6** | Thai text layer: word box จาก tesseract TSV, ToUnicode + ActualText, editable output ผ่าน Type 3 Thai | `crates/engine/src/ocr/layer.rs` | 10–18 h |
| **T7** | Segmentation ไทย (ICU4X) สำหรับ phrase grouping / ค้นหา / dictionary — ใช้ร่วมกับ Track B1 | `crates/engine/src/ocr/dictionary.rs` | 6–10 h |
| **T8** | Test corpus ไทยของเราเอง (public domain: ราชกิจจานุเบกษา, เอกสาร Open Data) + golden output | `tests/data/thai/` | 8–14 h |
| **T9** | Accuracy harness (CER/WER) ใช้ระเบียบวิธีจาก notebook (อ่านครั้งเดียวผ่าน subagent) | `tests/` | 6–10 h |
| **T10** | ATTRIBUTION.toml: tessdata_fast (Apache-2.0) + tesseract (Apache-2.0) + ระบุว่า mrolarik ใช้เป็น reference เท่านั้น | `ATTRIBUTION.toml` | 2–4 h |
| **T11** | Docs: OCR ไทยใช้ tesseract (ไม่ใช่ ocrs), ข้อจำกัด web build | `docs/*.md`, `crates/ocr/README.md` | 3–6 h |
| | **รวม** | | **56–100 h** |

**ตัดออกจากแผนเดิม (ประหยัดที่สุด):** D3 (ONNX Thai model) 40–80 h, D6 tuning 24–40 h
ที่ลดได้เพราะ tesseract + `tha` ให้คุณภาพดีอยู่แล้ว และไม่ต้องดูแลโมเดลเอง

### 2.3 ลำดับทำ Track D ใหม่
```
T1 (1-2h)  →  ทดสอบด้วยรูปของเราเอง 1 รูป  →  T2+T3 (โมเดล offline)
     →  T5 (UI)  →  T6 (text layer)  →  T8+T9 (corpus + accuracy)  →  T4,T7,T10,T11
```

---

## 3. Playbook ลด token (ใช้ได้กับทุก track)

### 3.1 กติกา 12 ข้อ

| # | มาตรการ | วิธีทำ | ประหยัด |
|---|---|---|---|
| 1 | **Graph-first** | `graphify query "..."` ก่อนแตะไฟล์; อ่านเฉพาะบรรทัดที่ query ชี้ | −25% |
| 2 | **Context capsule** | เริ่ม session ด้วย `docs/thai-context.md` ~1.5k tokens (แทนอ่าน plan 12k + README) | −10% |
| 3 | **Session ต่อ 1 PR** | จบเมื่อ context >60% แล้วเปิด session ใหม่ (กัน resend แบบ quadratic) | −15% |
| 4 | **Targeted test** | `cargo test -p pdfcraft-fonts shaping::` ไม่ใช่ `cargo test --workspace` | −8% |
| 5 | **Tail-only output** | `2>&1 \| tail -40` และ `-- -q` สำหรับ test | −5% |
| 6 | **แก้แบบ anchor edit** | `edit` แทนเขียนไฟล์ใหม่ทั้งไฟล์ (ไม่ต้อง re-emit โค้ดเดิม) | −10% |
| 7 | **Subagent สำหรับสำรวจ** | คืนสรุป ≤500 tokens; main session ไม่อ่านไฟล์ที่ subagent อ่านแล้ว | −12% |
| 8 | **Batch shell** | รวมคำสั่งอิสระใน 1 call (ไม่ใช่ 5 turn) | −5% |
| 9 | **Tiered model** | งานกลไก (TSV, docs, rename) ใช้โมเดลถูก; architecture ใช้โมเดลใหญ่ | cost −40% |
| 10 | **อ่าน notebook ครั้งเดียว** | subagent อ่าน `tesseract-ocr-Thai-language.ipynb` (792 KB) แล้วสรุป 20 บรรทัด | −3 M ต่อครั้ง |
| 11 | **Diff-only translation** | ส่งเฉพาะ key ที่ยังไม่มีคำแปล ทีละ 200 entries | −60% ของงาน i18n |
| 12 | **Decision log** | จดการตัดสินใจใน plan doc เพื่อไม่ต้องเถียงซ้ำ | −5% |

> มาตรการ 1,3,6,7 คือตัวหลัก — รวมกันลดได้ **~40–50%** ของ input tokens

### 3.2 โครงสร้าง session ที่แนะนำ

```
[Static prefix – cacheable]
  1. AGENTS.md (ย่อ)
  2. docs/thai-context.md        ← 1.5k tokens
  3. File ที่งานนี้แตะ (2–4 ไฟล์)
[Dynamic]
  4. graphify query result (≤2k)
  5. test output (tail 40 บรรทัด)
  6. งานที่ต้องทำ (1–3 ข้อ)
```

**เพดานต่อ session:** context ≤ 80k, output ≤ 4k, ไฟล์ที่อ่าน ≤ 6, test run ≤ 3

### 3.3 Context capsule (สร้างครั้งเดียว ~1.5k tokens)
`docs/thai-context.md` ควรมี:
- สถานะปัจจุบัน 8 บรรทัด (มี/ขาด)
- path ของไฟล์หลัก 10 บรรทัด
- DoD 6 ข้อ
- คำสั่ง test ที่ใช้บ่อย 5 บรรทัด
- ลิงก์ plan ฉบับเต็ม (ไม่ copy เนื้อหา)

---

## 4. งบ token ฉบับแก้ไข

| Track | ชั่วโมง (เดิม → ใหม่) | Input tokens (ใหม่) | Output |
|---|---|---|---|
| A ฟอนต์/เรนเดอร์ | 110–170 → 110–170 | 16–26 M | 0.35–0.6 M |
| B Layout | 60–95 → 60–95 | 9–15 M | 0.2–0.35 M |
| C UI ไทย | 80–130 → 80–130 | 11–20 M | 0.3–0.55 M |
| **D OCR ไทย** | **120–230 → 56–100** | **8–15 M** | **0.2–0.35 M** |
| E QA | 50–80 → 45–70 | 7–12 M | 0.2–0.35 M |
| **รวม** | **420–705 → 351–565** | **51–88 M** | **1.25–2.2 M** |
| + orchestration 15% | | **59–101 M** | **1.4–2.5 M** |

**เทียบกับแผนเดิม:** 150 M → **~75 M input (−50%)**, 3.5 M → **~2 M output (−43%)**

### 4.1 งบตาม milestone (ฉบับลด token)

| Milestone | เนื้องาน | Input | Output | Sessions |
|---|---|---|---|---|
| **M1** | T1,T2,T3,T5 + A1,A2 + corpus เริ่ม | **5–8 M** | 0.12–0.2 M | ~30 |
| **M2** | B1,B2,B3 + A5,A6,A7 + T6 | **17–28 M** | 0.35–0.6 M | ~120 |
| **M3** | C1–C3 + T4,T7 + A4 + accuracy | **22–38 M** | 0.55–0.9 M | ~150 |
| **M4** | T8–T11 + E2–E5 | **8–14 M** | 0.2–0.35 M | ~55 |

### 4.2 ค่าใช้จ่าย (โดยประมาณ)

| ระดับโมเดล | ค่าใช้จ่าย |
|---|---|
| Flash/mini class ($0.15/$0.60) | **$12–20** |
| กลาง ($0.5–1.0 / $2–5) | **$42–110** |
| Frontier ($3/$15) | **$250–460** |

พร้อม caching → ลดได้อีก 40–50%

---

## 5. ความเสี่ยงเฉพาะของแนวทาง tesseract

| ความเสี่ยง | ผล | มาตรการ |
|---|---|---|
| ผู้ใช้ไม่มี tesseract ติดตั้ง | OCR ไทยใช้ไม่ได้ | T3 + T5 (ปุ่ม/คู่มือ) + ระบุใน docs; `tesseract --list-langs` fail-closed ตามเดิม |
| มี `tha` แต่ใช้ `-l`/psm ผิด | คุณภาพต่ำ | T4 preset + T9 harness วัดจริง |
| Bundling 1.07 MB เพิ่มขนาดแอป | +1 MB | ยอมรับได้; ใช้ fast เป็นค่าเริ่มต้น, best เป็น opt-in |
| รูปทดสอบ mrolarik ไม่มี license | ผิดกฎหมายถ้า copy | ห้ามใช้; สร้าง corpus เอง (T8) |
| Thai ไม่มีช่องว่าง → word boxes | ค้นหา/แก้ไขยาก | T7 + Track B1 (ICU segmentation) |
| tesseract บน wasm | OCR ไทยบน web ไม่มี | ระบุข้อจำกัด; อนาคตค่อยดู ONNX |

---

## 6. Checklist ก่อนเริ่มแต่ละ session (ประหยัด token)

- [ ] เปิด `docs/thai-context.md` (ไม่ใช่ plan ฉบับเต็ม)
- [ ] รัน `graphify query` 1 ครั้งสำหรับงานนี้
- [ ] ระบุไฟล์ที่จะแตะ ≤ 4 ไฟล์
- [ ] ระบุ test command เดียวที่จะใช้
- [ ] กำหนด output ที่คาดหวัง ≤ 4k tokens
- [ ] ถ้า context >60% → หยุดและเปิด session ใหม่
