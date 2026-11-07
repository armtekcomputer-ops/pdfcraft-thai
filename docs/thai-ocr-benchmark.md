# Thai OCR: reference notebook evaluation

Reference: <https://github.com/mrolarik/Tesseract-Thai/blob/master/tesseract-ocr-Thai-language.ipynb>

## What the notebook is

A 2018-era demo of `pytesseract` on four sample images. Every code cell is the same
one-liner:

```python
pytesseract.image_to_string(Image.open(path), 'tha')
```

There is no preprocessing, no page-segmentation choice, no measured accuracy and no
language-pack staging: the notebook documents that `tesseract-ocr-tha` installs an
`tha` pack and that recognition then works. Its stored outputs were produced by the
Tesseract of that time and are visibly poor (for the textbook page it returns
"คา คิอถ้อยคาทึกล่าวออกมาให์รู้เฉพาหั้คำหนึ่งๆ" where the scan reads "คำ คือถ้อยคำที่
กล่าวออกมาให้รู้เฉพาะคำหนึ่งๆ").

**Licence: the repository has no LICENSE file.** Its images and text are not
redistributable, so none of them is vendored here and no test downloads them. This
note records a local evaluation only.

## What PdfCraft does instead

`crates/ocr/src/tesseract.rs` runs the installed binary with a fixed argv, a cleared
environment and the bundled language data:

```
tesseract <page.png> stdout -l tha+eng -c tessedit_create_tsv=1
```

`tha+eng` rather than `tha` (the Thai data alone reads `OCR` as `068` — see the
roadmap entry for 2026-11-07), TSV requested as a parameter (a bundled
`TESSDATA_PREFIX` has no `configs/`), and PST 3 (the default) is left alone.

## Local evaluation (2026-11-07)

Four images fetched by URL into a scratch directory (never committed), read with the
pipeline above and the `tessdata_fast` packs staged by `cargo xtask models`:

| Image | Size | Result |
|---|---|---|
| `BangkokThai_(vectorized).svg.png` | 2000×996 | Full ceremonial name in Thai **and** the English transliteration read correctly |
| `thai-text-005.jpg` (textbook page) | 638×903 | Near-perfect prose; only word-spacing and one `ที่`→`ที` error in the first lines |
| `thai-text.png` (synthetic specimen) | 460×502 | Partly readable; the distorted specimen defeats the recogniser |
| `thai-text-004.jpg` (nonsense glyphs, 300×184) | 300×184 | Unreadable by design; 2×/3×/4× Lanczos upscaling did not help |

Page-segmentation comparison on the two readable images: `--psm 3` and `--psm 4` were
identical; `--psm 6` joined lines and lost one word, so the default `psm 3` stays.

`thai-text-005.jpg` is the one real-prose sample, and the pipeline reads it far better
than the notebook's stored output, which is expected: that output predates the current
`tessdata_fast` Thai model.

## Conclusion

The notebook contributes no technique beyond `-l tha`, which PdfCraft already exceeds
(`tha+eng`, TSV, bundled packs, Thai word joining). No preset change follows from it,
and no evaluation accuracy number is claimed here because the sample set is four
images with no ground-truth transcriptions and no licence to redistribute them.

To reproduce locally:

```sh
cargo xtask models                      # stages assets/models/tessdata/{tha,eng}.traineddata
curl -LO https://raw.githubusercontent.com/mrolarik/Tesseract-Thai/master/data-text-img/thai-text-005.jpg
TESSDATA_PREFIX=$PWD/assets/models/tessdata \
  tesseract thai-text-005.jpg stdout -l tha+eng -c tessedit_create_tsv=1
```
