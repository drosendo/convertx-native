# Credits

ConvertX — Native would not exist without the work below. Everything here is
acknowledged with gratitude, and the licences of each project are respected.

## Upstream project

**[ConvertX](https://github.com/c4illin/ConvertX)** by **C4illin** and its contributors.

This project is a derivative work. The following were ported directly from the
upstream TypeScript implementation:

- `assets/formats.json` — the complete converter/format matrix (inputs, outputs,
  categories, install commands), extracted from upstream `src/converters/*.ts`
  and `src/main.ts` by `tools/gen_formats.py`
- The converter priority order and the same-category routing rule
  (`src/registry.rs`)
- The command-line arguments for every external converter (`src/convert.rs`)
- The LibreOffice filter tables for text and spreadsheet documents
- The extension normalisation rules (`normalizeFiletype.ts`)

Upstream is licensed under the **GNU Affero General Public License v3.0**, so
this project is released under the same licence. See [LICENSE](./LICENSE).

## External converters

ConvertX shells out to the best tool for each format. Each remains the property
of its authors, under its own licence.

| Converter | Project | Licence |
| --- | --- | --- |
| ImageMagick | [imagemagick.org](https://imagemagick.org) | ImageMagick License (Apache-2.0 compatible) |
| GraphicsMagick | [graphicsmagick.org](http://www.graphicsmagick.org) | MIT-style |
| FFmpeg | [ffmpeg.org](https://ffmpeg.org) | LGPL-2.1+ / GPL-2+ |
| LibreOffice | [libreoffice.org](https://www.libreoffice.org) | MPL-2.0 |
| Pandoc | [pandoc.org](https://pandoc.org) | GPL-2+ |
| Inkscape | [inkscape.org](https://inkscape.org) | GPL-2+ |
| libvips | [libvips.github.io/libvips](https://libvips.github.io/libvips/) | LGPL-2.1+ |
| resvg | [github.com/linebender/resvg](https://github.com/linebender/resvg) | MPL-2.0 |
| libjxl | [github.com/libjxl/libjxl](https://github.com/libjxl/libjxl) | BSD-3-Clause |
| libheif | [github.com/strukturag/libheif](https://github.com/strukturag/libheif) | LGPL-3.0 |
| Calibre | [calibre-ebook.com](https://calibre-ebook.com) | GPL-3.0 |
| XeLaTeX / TeX Live | [tug.org/texlive](https://tug.org/texlive/) | LPPL / GPL |
| DjVuLibre | [djvu.sourceforge.net](http://djvu.sourceforge.net) | GPL-2+ |
| Poppler (`pdftotext`, `pdftoppm`, `pdftops`) | [poppler.freedesktop.org](https://poppler.freedesktop.org) | GPL-2+ |
| Tesseract OCR | [github.com/tesseract-ocr](https://github.com/tesseract-ocr/tesseract) | Apache-2.0 |
| Assimp | [assimp.org](https://www.assimp.org) | BSD-3-Clause |
| Potrace | [potrace.sourceforge.net](https://potrace.sourceforge.net) | GPL-2+ |
| VTracer | [github.com/visioncortex/vtracer](https://github.com/visioncortex/vtracer) | MIT |
| dasel | [github.com/TomWright/dasel](https://github.com/TomWright/dasel) | MIT |
| msgconvert | [github.com/mvz/email-outlook-message-perl](https://github.com/mvz/email-outlook-message-perl) | GPL-1+ |
| markitdown | [github.com/microsoft/markitdown](https://github.com/microsoft/markitdown) | MIT |
| sips / textutil | Apple macOS | Bundled with macOS |

Optional accelerators for the image optimizer: [jpegtran / mozjpeg](https://github.com/mozilla/mozjpeg)
(IJW), [gifsicle](https://www.lcdf.org/gifsicle/) (GPL-2), [cwebp / libwebp](https://developers.google.com/speed/webp)
(BSD-3), [SVGO](https://github.com/svg/svgo) (MIT), [svgcleaner](https://github.com/RazrFalcon/svgcleaner) (GPL-3),
[pngquant](https://pngquant.org) (GPL-3+), [oxipng](https://github.com/shssoichiro/oxipng) (MIT).

## Rust crates

| Crate | Used for | Licence |
| --- | --- | --- |
| [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui) | desktop interface | MIT / Apache-2.0 |
| [image](https://github.com/image-rs/image) | built-in image codecs | MIT / Apache-2.0 |
| [oxipng](https://github.com/shssoichiro/oxipng) | lossless PNG optimisation | MIT |
| [resvg](https://github.com/linebender/resvg) / [usvg](https://github.com/linebender/resvg) / [tiny-skia](https://github.com/linebender/tiny-skia) | SVG rasterisation | MPL-2.0 |
| [calamine](https://github.com/tafia/calamine) | spreadsheet reading | MIT |
| [zip](https://github.com/zip-rs/zip2) | DOCX/ODT/EPUB extraction | MIT |
| [rfd](https://github.com/PolyMeilex/rfd) | native file dialogs | MIT |
| [serde](https://serde.rs) / [serde_json](https://github.com/serde-rs/json) | settings, history, registry | MIT / Apache-2.0 |
| [chrono](https://github.com/chronotope/chrono) | history timestamps | MIT / Apache-2.0 |
| [which](https://github.com/harryfei/which-rs) | tool discovery | MIT |

## Format knowledge

The list of supported formats, and the knowledge of which tool handles which
pair, comes from the upstream project and the documentation of the tools above.
Typeface, icon and colour choices in the interface are original to this project.
