# Thai OCR test fixtures — attribution

These files are small, licence-clean fixtures for exercising the Thai OCR path
(`tests/data/thai_ocr/`, used by `cargo xtask ocr-bench`). They are **test data
only** and are not shipped in release artefacts, packages or app bundles.

## Source document

- Title: *กฎบัตรอาเซียน* (ASEAN Charter), Thai translation — the treaty text
  adopted at Singapore on 20 November 2007.
- Original PDF (331,440 bytes, 68 pages, A4, digital text + scan):
  <https://upload.wikimedia.org/wikipedia/commons/1/14/ASEAN_Charter_Thai.pdf>
- Wikimedia Commons file page:
  <https://commons.wikimedia.org/wiki/File:ASEAN_Charter_Thai.pdf>
- Transcription index on Thai Wikisource:
  <https://th.wikisource.org/wiki/ดัชนี:ASEAN_Charter_Thai.pdf>

## Licence

- **Original document:** the ASEAN Charter is an official treaty text — an
  official document of a legislative/administrative nature, published by the
  governments of the ASEAN member states. Thai copyright law (Copyright Act
  B.E. 2537 (1994), s. 7) excludes official texts of that kind from protection,
  so the charter itself is treated as **public domain / official document**.
- **Wikisource transcription:** the page-by-page proofread text on Thai
  Wikisource is a collaborative transcription released under
  **CC BY-SA 4.0** (<https://creativecommons.org/licenses/by-sa/4.0/>).
  Reuse of the `.txt` fixtures must credit Thai Wikisource and keep the same
  licence. The underlying treaty text itself remains an official document.
- All six transcriptions used here carry `<pagequality level="3" />`
  (proofread / "ตรวจทานแล้ว"). Page 3–8 were confirmed level 3 at fetch time;
  the transcriptions were made by the Wikisource user `Horus`.

## Pages used (PDF pages 3–8 = Wikisource pages 3–8)

| Fixture | Wikisource page |
| --- | --- |
| `page-03.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/3> |
| `page-04.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/4> |
| `page-05.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/5> |
| `page-06.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/6> |
| `page-07.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/7> |
| `page-08.png` / `.txt` | <https://th.wikisource.org/wiki/หน้า:ASEAN_Charter_Thai.pdf/8> |

Raw text was fetched through the MediaWiki API (requires a real User-Agent):

```sh
curl -sS -A "ThaiOCRTestFixture/1.0" \
  "https://th.wikisource.org/w/api.php?action=query&titles=<url-encoded page title>&prop=revisions&rvprop=content&rvslots=main&format=json"
```

## Rasterisation

```sh
pdftoppm -f 3 -l 8 -r 200 -gray -png asean.pdf page
```

This produced 200 dpi, 8-bit grayscale PNGs (1654 × 2339 px, A4,
190–288 KB each), renamed to `page-03.png` … `page-08.png`.

## Ground-truth text

`page-NN.txt` is the proofread wikitext with presentation markup removed the way
a human reads the rendered page:

- `<noinclude>…</noinclude>` blocks dropped (they hold the page-number header
  template, e.g. `{{กลาง|4}}`);
- `{{กลาง|X}}` — a centre-alignment wrapper, not content — reduced to its
  visible argument `X`, so the chapter/article headings
  ("หมวดที่ 1", "ความมุ่งประสงค์และหลักการ", "ข้อ 1"…) survive;
- all other `{{…}}` templates dropped (notably `{{nop}}`);
- HTML comments and remaining `<…>` tags dropped;
- bold/italic quotes dropped, `[[A|B]]` → `B`, `[[A]]` → `A`;
- leading `*` / `#` / `:` list markers dropped;
- spaces collapsed and lines trimmed, line breaks and blank lines kept.

Apart from those presentation rules, Thai spelling, digits and punctuation are
left exactly as transcribed — no transliteration, no spelling correction, no
re-wrapping of the scan's line breaks. This includes the source's old
decomposed spelling (`ทํา` = U+0E4D + U+0E32 rather than U+0E33); the benchmark
compares both sides after NFKC, so it costs nothing.

One deliberate exception: a single stray private-use codepoint U+F70A on page 6,
where the scan plainly prints `่` (U+0E48 mai ek) in "แห่ง", is normalised to
U+0E48. It is a transcription typo, not visible text.

Cleaning was done by a small throwaway script, not by hand, so the result is
reproducible from the API text.
