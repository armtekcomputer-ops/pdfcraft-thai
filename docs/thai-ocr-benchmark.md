# Thai OCR: benchmark corpus and reference notebook evaluation

PdfCraft measures Thai OCR with a licence-clean corpus committed to this
repository, scoring the app's two offline Thai readers: the built-in PaddleOCR
engine and the Tesseract invocation the app issues when Tesseract is chosen. The
mrolarik notebook below is retained only as a note on a route that was evaluated
and not used: it has no licence and teaches no technique.

## The benchmark corpus

`tests/data/thai_ocr/` holds six pages of the Thai translation of the **ASEAN
Charter** (PDF pages 3–8), paired with proofread transcriptions:

| Page | Reference characters | Paddle edits | Paddle CER | Tesseract edits | Tesseract CER |
|---|---:|---:|---:|---:|---:|
| `page-03.png` | 847 | 4 | 0.47% | 10 | 1.18% |
| `page-04.png` | 522 | 1 | 0.19% | 6 | 1.15% |
| `page-05.png` | 776 | 8 | 1.03% | 17 | 2.19% |
| `page-06.png` | 831 | 9 | 1.08% | 18 | 2.17% |
| `page-07.png` | 696 | 1 | 0.14% | 27 | 3.88% |
| `page-08.png` | 527 | 4 | 0.76% | 11 | 2.09% |
| **total** | **4,199** | **27** | **0.64%** | **89** | **2.12%** |

Measured 2026-11-08 on this machine with `cargo xtask ocr-bench --engine paddle`
and `cargo xtask ocr-bench --engine tesseract`; both runs reported
`PASS (total CER 0.64% <= target 10.00%)` and
`PASS (total CER 2.12% <= target 10.00%)` respectively.

Page 04 was the hardest page of the earlier three-page trial that motivated the
corpus. That trial predates the current pipeline, and its number is not
reproducible with the commands below, so it is not repeated here; in the
six-page run every page is under 4% with Tesseract and under 1.1% with Paddle,
and page 04 is 1.15% with Tesseract and 0.19% with Paddle.

### Rasterisation

The pages were rendered from the Wikimedia Commons PDF
(`ASEAN_Charter_Thai.pdf`) at 200 dpi, 8-bit grayscale:

```sh
pdftoppm -f 3 -l 8 -r 200 -gray -png asean.pdf page
```

That produces 1654 × 2339 px A4 pages (190–288 KB each), renamed
`page-03.png` … `page-08.png`.

### Ground truth

`page-NN.txt` is the Thai Wikisource proofread text (`<pagequality level="3" />`,
"ตรวจทานแล้ว") with presentation markup removed the way a reader sees the page:
`<noinclude>` blocks, `{{nop}}` and other templates dropped, `{{กลาง|X}}`
reduced to its visible argument, markup and list prefixes removed, lines
trimmed. Thai spelling, digits and punctuation are left as transcribed; the
source's old decomposed spelling (`ทํา`, U+0E4D + U+0E32) is kept and folds
away in the comparison. One private-use codepoint (U+F70A) that stands for
U+0E48 on page 6 is the only correction.

### Licence and attribution

- The charter is an official treaty text, an official document of a
  legislative/administrative nature. Thai copyright law (Copyright Act B.E.
  2537 (1994), s. 7) excludes such texts from protection, so the underlying
  document is treated as public domain.
- The Wikisource transcription is collaborative work released under
  **CC BY-SA 4.0**; the `.txt` fixtures must credit Thai Wikisource and keep
  that licence. `tests/data/thai_ocr/LICENSE-CC-BY-SA-4.0.txt` is the licence
  text.
- `tests/data/thai_ocr/ATTRIBUTION.md` records the source, licence, page links
  and cleaning rules. Each PNG has exactly one `[[asset]]` entry in
  `ATTRIBUTION.toml` with its SHA-256.
- These files are test data only. They are not shipped in release artefacts,
  packages or app bundles.

## Running the benchmark

```sh
cargo xtask ocr-bench --engine paddle     # the built-in PaddleOCR reader
cargo xtask ocr-bench --engine tesseract  # comparison against Tesseract
cargo xtask ocr-bench --target 5          # a stricter pass mark
cargo xtask ocr-bench path/to/pages       # another PNG + sibling .txt corpus
```

For every `*.png` with a sibling `*.txt` of the same stem, the chosen engine
reads the page.

`--engine tesseract` (the default, and the behaviour before the flag existed)
runs the app's invocation:

```
tesseract <page.png> stdout -l tha+eng -c tessedit_create_tsv=1
```

with `TESSDATA_PREFIX` pointing at the bundled `assets/models/tessdata` when
that directory exists (the caller's environment is otherwise left alone). It
joins the level-5 words of the TSV with spaces.

`--engine paddle` measures the app's own `pdfcraft_ocr::paddle` engine without
linking it into xtask: it builds a one-page PDF from the PNG and drives
`pdfcraft-cli run --script` through `doc_create`, `ocr_recognize` (`language`
`th`, `engine` `paddle`) and `doc_export_text`, then scores the exported text
exactly like the Tesseract path. The CLI is `$PDFCRAFT_CLI` or
`target/debug/pdfcraft-cli`; when the program, its paddle engine or the three
models under `assets/models` are missing the command prints a `skipped` line and
exits zero, like a missing `tesseract`.

Each engine's output is normalised, its Levenshtein distance to the transcript is
computed in characters, and the command prints a per-page and a total character
error rate. The total sums the edits and the reference lengths before dividing,
so long pages weigh more. It exits non-zero when the total exceeds `--target`
(default 10%).

### What the metric folds

- **Whitespace is ignored on both sides** after normalisation. Thai does not
  space every word, and an engine may report each cluster or each line as a
  "word", so joining with spaces would invent boundaries; spacing choices cost
  nothing.
- **NFKC is applied to both sides.** It folds Thai SARA AM (U+0E33) into
  NIKHAHIT (U+0E4D) plus SARA AA (U+0E32), which is what the recogniser emits,
  so pre-composed and decomposed spellings of the same word compare equal.
  NIKHAHIT is then moved back before a preceding tone mark, the order Thai
  writes and the recogniser emits.
- The corpus spelling is old/decomposed in places; NFKC means that costs
  nothing.

### What the metric does not do

- **No spelling correction, normalisation to a dictionary word list, or
  semantic scoring.** A recognised word must match the transcript character for
  character after the folds above; plausible but differently spelled Thai
  counts as an edit.
- **No layout or reading-order scoring.** Only the joined word text is compared;
  block/line/word boxes, confidence and reading order are not measured.
- **No corpus-wide generalisation claim.** Six clean 200 dpi pages of one
  document are not 300/150 dpi, skewed, photocopied or photographed pages. The
  number is a regression gate for this pipeline, not an accuracy promise.

## The mrolarik notebook

Reference: <https://github.com/mrolarik/Tesseract-Thai/blob/master/tesseract-ocr-Thai-language.ipynb>

A 2018-era demo of `pytesseract` on four sample images. Every code cell is the
same one-liner:

```python
pytesseract.image_to_string(Image.open(path), 'tha')
```

There is no preprocessing, no page-segmentation choice, no measured accuracy
and no language-pack staging: the notebook documents that
`tesseract-ocr-tha` installs a `tha` pack and that recognition then works. Its
stored outputs were produced by the Tesseract of that time and are visibly poor
(for the textbook page it returns "คา คิอถ้อยคาทึกล่าวออกมาให์รู้เฉพาหั้คำหนึ่งๆ"
where the scan reads "คำ คือถ้อยคำที่กล่าวออกมาให้รู้เฉพาะคำหนึ่งๆ").

**Licence: the repository has no LICENSE file.** Its images and text are not
redistributable, so none of them is vendored here and no test downloads them.
This note records a local evaluation only.

A local trial found the pipeline above reads the one real-prose sample far
better than the notebook's stored output, as expected: that output predates the
current `tessdata_fast` Thai model. The notebook contributes no technique beyond
`-l tha`, which PdfCraft already exceeds (`tha+eng`, TSV, bundled packs, Thai
word joining). No evaluation number is claimed from it, because the sample set
has no ground-truth transcriptions and no licence to redistribute it.
