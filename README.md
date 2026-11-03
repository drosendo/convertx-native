# ConvertX — native file conversion for your desktop

[![Licence: AGPL v3](https://img.shields.io/badge/licence-AGPL--3.0-blue.svg)](./LICENSE)
[![Built with Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-34%20passing-brightgreen.svg)](#building)

A native rewrite of [ConvertX](https://github.com/c4illin/ConvertX) in Rust, with a desktop GUI and a
scriptable command line. It converts files between 900+ formats and optimizes images, reusing the
converters you already have installed — and falling back to built-in Rust engines when you have none.

No server, no browser tab, no account.

## Download

Prebuilt releases are on the [releases page](https://github.com/drosendo/convertx-native/releases):

| Platform | File | Notes |
| --- | --- | --- |
| macOS 11+ (Apple silicon) | `ConvertX-*-macos-arm64.zip` | unzip, drag into `/Applications` |
| macOS 11+ (Intel) | `ConvertX-*-macos-x64.zip` | same |
| Windows 10+ | `convertx-*-windows-x64.zip` | unzip, run `convertx.exe` |
| Linux (glibc) | `convertx-*-linux-x64.tar.gz` | `tar xzf`, run `./convertx` |
| Android | built from source — see [Android](#android) | built-in engines only |

## What makes it reliable

| Problem | How ConvertX handles it |
| --- | --- |
| A converter is missing | Built-in Rust engines cover images, SVG, spreadsheets and documents with zero installs |
| A converter is installed but fails on a file | The next converter that supports the route is tried automatically, and the winner is reported |
| A converter exits 0 but writes nothing | Every output is validated: file exists, is non-empty, and images/PDFs actually parse |
| Phone photos come out sideways | EXIF orientation is applied on read and preserved through conversion |
| GUI launch can't find Homebrew tools | `PATH` is rebuilt with Homebrew, MacPorts, TeX Live and `~/.local/bin` before probing |
| A tool lives somewhere unusual | Pick it by hand under Settings → Locate…; the override wins over auto-detection |
| `HEIC` → one JPG gives you three files | Only the primary frame of photo containers is read |
| A converter hangs forever | A watchdog stops it after the configured limit, so the batch keeps going |
| `jpg` and `jpeg` were treated as different formats | All aliases fold into one canonical name, so no converter is skipped |

## Conversion

Drop files in, choose a target format per file (or one format for everything), press Convert.
Converters run in parallel; each file reports the converter that actually produced it.

### Built-in engines — no install required

| Engine | Reads | Writes |
| --- | --- | --- |
| Image codecs (`image`) | PNG, JPEG, GIF, BMP, TIFF, ICO, WebP, TGA, PNM, QOI, DDS, HDR, EXR | the same set (WebP is lossless) |
| SVG rasterizer (`resvg`) | SVG (with system fonts) | PNG, JPEG, WebP, TIFF, ICO, BMP, TGA, QOI |
| Spreadsheet reader (`calamine`) | XLSX, XLSM, XLS, XLSB, ODS, CSV, TSV | CSV, TSV, JSON |
| Document reader | DOCX, ODT, EPUB, RTF, HTML, plain text | TXT, Markdown, HTML |
| Contacts | VCF | CSV |

### External converters — used when installed

ImageMagick, GraphicsMagick, FFmpeg, LibreOffice, Pandoc, Inkscape, libvips, resvg, libjxl, libheif,
Calibre, XeLaTeX, DjVu, Poppler (`pdftotext`, `pdftoppm`, `pdftops`), Tesseract OCR, Assimp, Potrace,
VTracer, dash-dot (`dasel`), `msgconvert`, `markitdown`.

On macOS, the built-in `sips` (HEIC/HEIF, RAW photos) and `textutil` (RTF, DOC, HTML) are detected
automatically, so photos and office text work out of the box.

Install what you're missing from **Settings → Converters**: every converter shows whether it is
available, which file provides it, and the exact install command — nothing is installed without you
clicking **Install**.

## Image optimization

Switch the header to **Optimize**. It removes bloat and recompresses, but by default not a single
pixel changes.

| Format | Lossless optimization |
| --- | --- |
| PNG | `oxipng`: filter selection, palette/bit-depth reduction, zopfli deflate, metadata chunks removed |
| JPEG | EXIF/IPTC/XMP/comments/thumbnails stripped, Huffman tables rebuilt with `jpegtran` when available |
| WebP | `EXIF` and `XMP` RIFF chunks dropped |
| GIF | `gifsicle -O3 --careful` when present, otherwise comment and application blocks removed |
| SVG | comments, `<metadata>`, Inkscape/Sodipodi bookkeeping and inter-tag whitespace removed (or `svgo`/`svgcleaner`) |
| TIFF/AVIF/HEIC | ImageMagick `-strip` when installed |

Three modes:

- **Lossless** — pixel-identical, metadata gone, smallest lossless encoding. The default.
- **Balanced** — re-encodes at quality 82. Visually identical for most photographs.
- **Smallest** — quality 70.

Optional: shrink the long edge, keep a `.orig` backup, or overwrite the originals instead of writing
copies. The ICC colour profile is always kept in lossless mode because dropping it shifts colours; the
optimizer refuses to hand back a file larger than the original.

## Command line

```bash
convertx                                   # open the window
convertx report.docx notes.odt             # open the window with files queued

convertx --to pdf report.docx              # convert headlessly
convertx --to jpg ~/Pictures/*.heic
convertx --to txt scan.png                 # OCR via Tesseract
convertx --to md report.docx               # built-in extractor, no LibreOffice needed

convertx --optimize --in-place ~/site/assets/*.png
convertx --optimize --quality 80 --max 1600 photo.jpg

convertx --list                            # converters and their status
convertx --help
```

Exit code is `0` when every file succeeded and `1` when any file failed, so it drops into scripts and
CI without extra glue.

## Keyboard

| Shortcut | Action |
| --- | --- |
| `⌘/Ctrl + O` | add files |
| `⌘/Ctrl + Enter` | start the current mode |
| `⌘/Ctrl + 1` / `⌘/Ctrl + 2` | switch between Convert and Optimize |

## Building

Requires Rust 1.85 or newer (edition 2024).

```bash
cargo build --release
./target/release/convertx
```

The release profile enables LTO and strips symbols; the binary embeds the whole format registry, so
it is a single self-contained file.

```bash
cargo test    # 34 tests: real conversions, optimisation, routing and UI smoke tests
```

### macOS app bundle

```bash
./tools/package_macos.sh      # builds dist/ConvertX.app with icon and version info
open dist/ConvertX.app
```

Then drag `dist/ConvertX.app` into `/Applications` so it shows up in Launchpad,
Spotlight and the Dock. The bundle is signed ad-hoc, which is enough for local
use; Gatekeeper will still warn on a machine that did not build it (right-click →
Open the first time).

You can also launch without the bundle:

```bash
cargo run --release            # or: ./target/release/convertx
```

## Configuration

Settings live in `~/Library/Application Support/ConvertX/settings.json` on macOS,
`%APPDATA%\ConvertX\settings.json` on Windows and `~/.config/convertx/settings.json` elsewhere.

| Key | Meaning |
| --- | --- |
| `output_dir` | where converted files go |
| `parallel_jobs` | worker threads (1–8) |
| `jpeg_quality` | JPEG encoder quality for conversions |
| `svg_scale` | raster scale for SVG output |
| `ffmpeg_input_args` / `ffmpeg_output_args` | extra FFmpeg flags |
| `custom_paths` | binary name → absolute path, from Settings → Locate… |
| `convert_timeout_secs` | watchdog limit per conversion; `0` disables it |

## Project layout

```
src/app.rs        egui interface: queue, format picker, optimize panel, settings, history
src/convert.rs    command building for every external converter, fallback chain, output validation
src/native/       built-in Rust engines: images, svg, sheets, docs
src/optimize.rs   lossless image optimizer
src/registry.rs   format routing from the embedded JSON tables
src/tools.rs      tool discovery, version probing, package manager mapping
src/cli.rs        headless mode
assets/           formats.json (ported from upstream) + native.json (built-in engines)
tools/            gen_formats.py — regenerates formats.json from an upstream checkout
```

## Android

The Android build compiles the same Rust core and the same egui interface, but **only the built-in
engines work**: Android forbids spawning the external tools, so ImageMagick, FFmpeg, LibreOffice and
friends are unavailable. Images, SVG, spreadsheets, documents, VCF and the whole image optimizer
run in-process and need no installs.

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME="$HOME/Library/Android/sdk/ndk/27.2.12479018"
./tools/package_android.sh          # builds dist/ConvertX.apk
adb install -r dist/ConvertX.apk
```

The resulting APK is debug-signed, so it installs directly; publishing to Play would need a
release keystore.

## Contributing

Everyone is welcome — bug reports, converter requests, docs and code.
[`good first issue`](https://github.com/drosendo/convertx-native/labels/good%20first%20issue)
labels the small ones.

- [CONTRIBUTING.md](./CONTRIBUTING.md) — setup, the checks CI runs, and step-by-step guides for
  adding a converter or a built-in engine
- [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)
- [CREDITS.md](./CREDITS.md) — who built what this project stands on

Adding a converter usually needs no Rust at all if the tool takes a simple `in → out` command:
[`assets/native.json`](./assets/native.json) declares it, and one match arm in
[`src/convert.rs`](./src/convert.rs) wires the argv.

## Credits and licence

Built on the shoulders of others — the full list is in [CREDITS.md](./CREDITS.md).

**Upstream:** this is a native rewrite of [ConvertX](https://github.com/c4illin/ConvertX) by
[C4illin](https://github.com/c4illin). The converter matrix, converter priority order, routing rules
and command-line arguments are ported from that project, which is licensed under the
**GNU Affero General Public License v3.0**. This project is therefore a derivative work and is
released under the same licence — see [LICENSE](./LICENSE).

**Converters:** ImageMagick, FFmpeg, LibreOffice, Pandoc, Inkscape, libvips, resvg, libjxl, libheif,
Calibre, XeLaTeX, DjVuLibre, Poppler, Tesseract, Assimp, Potrace, VTracer, dasel, msgconvert,
markitdown, and Apple's `sips`/`textutil`. Each stays the property of its authors under its own
licence; ConvertX only orchestrates them.

**Rust crates:** `egui`/`eframe` (MIT/Apache-2.0), `image` (MIT/Apache-2.0), `oxipng` (MIT),
`resvg`/`usvg`/`tiny-skia` (MPL-2.0), `calamine` (MIT), `zip` (MIT), `rfd` (MIT),
`serde`/`serde_json` (MIT/Apache-2.0), `chrono` (MIT/Apache-2.0), `which` (MIT).
