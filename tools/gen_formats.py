#!/usr/bin/env python3
"""Generate assets/formats.json from the upstream ConvertX converter definitions.

Usage:
    python3 tools/gen_formats.py /path/to/ConvertX

The upstream project (https://github.com/C4illin/ConvertX, AGPL-3.0) keeps one
TypeScript module per converter under ``src/converters``. Each module exports a
``properties`` object describing which formats it can read and write, grouped by
category (images, document, text, ...). This script extracts those tables and
merges them with the desktop-app metadata (binaries, packages, options) below so
the Rust binary has a single self-contained registry.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

# Registry order mirrors upstream `src/converters/main.ts`: the first converter
# that supports a from/to pair within the same category wins.
CONVERTERS = [
    {
        "id": "inkscape",
        "name": "Inkscape",
        "binaries": ["inkscape"],
        "description": "Vector graphics (SVG, EPS, PDF, EMF/WMF)",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "--cask", "inkscape"],
            "apt": ["install", "-y", "inkscape"],
            "dnf": ["install", "-y", "inkscape"],
            "pacman": ["-S", "--noconfirm", "inkscape"],
            "winget": ["install", "--id", "Inkscape.Inkscape"],
            "choco": ["install", "-y", "inkscape"],
        },
    },
    {
        "id": "libjxl",
        "name": "libjxl",
        "binaries": ["cjxl", "djxl"],
        "description": "JPEG XL encoder and decoder",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "jpeg-xl"],
            "apt": ["install", "-y", "libjxl-tools"],
            "dnf": ["install", "-y", "libjxl-utils"],
            "pacman": ["-S", "--noconfirm", "libjxl"],
        },
    },
    {
        "id": "resvg",
        "name": "resvg",
        "binaries": ["resvg"],
        "description": "SVG rasterizer",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "resvg"],
            "cargo": ["install", "resvg"],
            "pacman": ["-S", "--noconfirm", "resvg"],
        },
    },
    {
        "id": "vips",
        "name": "libvips",
        "binaries": ["vips"],
        "description": "Fast image processing library",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "vips"],
            "apt": ["install", "-y", "libvips-tools"],
            "dnf": ["install", "-y", "vips-tools"],
            "pacman": ["-S", "--noconfirm", "libvips"],
        },
    },
    {
        "id": "libheif",
        "name": "libheif",
        "binaries": ["heif-convert"],
        "description": "HEIC / HEIF images",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "libheif"],
            "apt": ["install", "-y", "libheif-examples"],
            "dnf": ["install", "-y", "libheif-tools"],
            "pacman": ["-S", "--noconfirm", "libheif"],
        },
    },
    {
        "id": "xelatex",
        "name": "XeLaTeX",
        "binaries": ["latexmk", "xelatex"],
        "description": "LaTeX typesetting",
        "version_args": ["-v"],
        "install": {
            "brew": ["install", "--cask", "basictex"],
            "apt": ["install", "-y", "texlive-xetex", "latexmk"],
            "dnf": ["install", "-y", "texlive-xetex", "latexmk"],
            "pacman": ["-S", "--noconfirm", "texlive-xetex", "texlive-binextra"],
        },
    },
    {
        "id": "djvu",
        "name": "DjVuLibre",
        "binaries": ["ddjvu"],
        "description": "DjVu documents",
        "version_args": ["--help"],
        "install": {
            "brew": ["install", "djvulibre"],
            "apt": ["install", "-y", "djvulibre-bin"],
            "dnf": ["install", "-y", "djvulibre"],
            "pacman": ["-S", "--noconfirm", "djvulibre"],
        },
    },
    {
        "id": "calibre",
        "name": "Calibre",
        "binaries": ["ebook-convert"],
        "description": "E-books (EPUB, MOBI, AZW3, ...)",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "--cask", "calibre"],
            "apt": ["install", "-y", "calibre"],
            "dnf": ["install", "-y", "calibre"],
            "pacman": ["-S", "--noconfirm", "calibre"],
            "winget": ["install", "--id", "calibre.calibre"],
        },
    },
    {
        "id": "dasel",
        "name": "Dasel",
        "binaries": ["dasel"],
        "description": "JSON, YAML, TOML, XML and CSV",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "dasel"],
            "go": ["install", "github.com/TomWright/dasel/v2/cmd/dasel@latest"],
            "pacman": ["-S", "--noconfirm", "dasel"],
        },
    },
    {
        "id": "libreoffice",
        "name": "LibreOffice",
        "binaries": ["soffice"],
        "description": "Office documents and spreadsheets",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "--cask", "libreoffice"],
            "apt": ["install", "-y", "libreoffice"],
            "dnf": ["install", "-y", "libreoffice"],
            "pacman": ["-S", "--noconfirm", "libreoffice-fresh"],
            "winget": ["install", "--id", "TheDocumentFoundation.LibreOffice"],
            "choco": ["install", "-y", "libreoffice-fresh"],
        },
    },
    {
        "id": "pandoc",
        "name": "Pandoc",
        "binaries": ["pandoc"],
        "description": "Markup and document formats",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "pandoc"],
            "apt": ["install", "-y", "pandoc"],
            "dnf": ["install", "-y", "pandoc"],
            "pacman": ["-S", "--noconfirm", "pandoc-cli"],
            "winget": ["install", "--id", "JohnMacFarlane.Pandoc"],
            "choco": ["install", "-y", "pandoc"],
        },
    },
    {
        "id": "msgconvert",
        "name": "msgconvert",
        "binaries": ["msgconvert"],
        "description": "Outlook .msg to .eml",
        "version_args": ["--help"],
        "install": {
            "cpan": ["install", "Email::Outlook::Message"],
        },
        "manual": "Install with: cpan install Email::Outlook::Message (Perl)",
    },
    {
        "id": "dvisvgm",
        "name": "dvisvgm",
        "binaries": ["dvisvgm"],
        "description": "DVI / EPS / PDF to SVG",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "dvisvgm"],
            "apt": ["install", "-y", "dvisvgm"],
            "dnf": ["install", "-y", "dvisvgm"],
            "pacman": ["-S", "--noconfirm", "dvisvgm"],
        },
    },
    {
        "id": "imagemagick",
        "name": "ImageMagick",
        "binaries": ["magick"],
        "description": "Raster images, hundreds of formats",
        "version_args": ["-version"],
        "install": {
            "brew": ["install", "imagemagick"],
            "apt": ["install", "-y", "imagemagick"],
            "dnf": ["install", "-y", "ImageMagick"],
            "pacman": ["-S", "--noconfirm", "imagemagick"],
            "winget": ["install", "--id", "ImageMagick.ImageMagick"],
            "choco": ["install", "-y", "imagemagick"],
        },
    },
    {
        "id": "graphicsmagick",
        "name": "GraphicsMagick",
        "binaries": ["gm"],
        "description": "Image processing toolkit",
        "version_args": ["version"],
        "install": {
            "brew": ["install", "graphicsmagick"],
            "apt": ["install", "-y", "graphicsmagick"],
            "dnf": ["install", "-y", "GraphicsMagick"],
            "pacman": ["-S", "--noconfirm", "graphicsmagick"],
        },
    },
    {
        "id": "assimp",
        "name": "Assimp",
        "binaries": ["assimp"],
        "description": "3D models and scenes",
        "version_args": ["version"],
        "install": {
            "brew": ["install", "assimp"],
            "apt": ["install", "-y", "assimp-utils"],
            "dnf": ["install", "-y", "assimp"],
            "pacman": ["-S", "--noconfirm", "assimp"],
        },
    },
    {
        "id": "ffmpeg",
        "name": "FFmpeg",
        "binaries": ["ffmpeg"],
        "description": "Video, audio and image sequences",
        "version_args": ["-version"],
        "install": {
            "brew": ["install", "ffmpeg"],
            "apt": ["install", "-y", "ffmpeg"],
            "dnf": ["install", "-y", "ffmpeg"],
            "pacman": ["-S", "--noconfirm", "ffmpeg"],
            "winget": ["install", "--id", "Gyan.FFmpeg"],
            "choco": ["install", "-y", "ffmpeg"],
        },
    },
    {
        "id": "potrace",
        "name": "Potrace",
        "binaries": ["potrace"],
        "description": "Bitmap to vector tracing",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "potrace"],
            "apt": ["install", "-y", "potrace"],
            "dnf": ["install", "-y", "potrace"],
            "pacman": ["-S", "--noconfirm", "potrace"],
            "choco": ["install", "-y", "potrace"],
        },
    },
    {
        "id": "vtracer",
        "name": "VTracer",
        "binaries": ["vtracer"],
        "description": "Raster to SVG tracing",
        "version_args": ["--version"],
        "install": {
            "brew": ["install", "vtracer"],
            "cargo": ["install", "vtracer"],
        },
    },
    {
        "id": "vcf",
        "name": "VCF to CSV",
        "binaries": [],
        "builtin": True,
        "description": "Contact cards (built in)",
        "version_args": [],
        "install": {},
    },
    {
        "id": "markitdown",
        "name": "MarkItDown",
        "binaries": ["markitdown"],
        "description": "Documents to Markdown",
        "version_args": ["--version"],
        "install": {
            "pipx": ["install", "markitdown[all]"],
            "pip": ["install", "--user", "markitdown[all]"],
            "brew": ["install", "markitdown"],
        },
    },
    {
        "id": "pdftops",
        "name": "pdftops (Poppler)",
        "binaries": ["pdftops"],
        "description": "PDF to PostScript / EPS",
        "version_args": ["-v"],
        "install": {
            "brew": ["install", "poppler"],
            "apt": ["install", "-y", "poppler-utils"],
            "dnf": ["install", "-y", "poppler-utils"],
            "pacman": ["-S", "--noconfirm", "poppler"],
            "winget": ["install", "--id", "oschwartz10612.Poppler"],
        },
    },
]


def extract_properties(path: pathlib.Path) -> dict:
    src = path.read_text()
    match = re.search(r"export const properties\s*=\s*", src)
    if not match:
        raise SystemExit(f"no properties export in {path}")
    start = match.end()
    depth = 0
    end = start
    while end < len(src):
        char = src[end]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                break
        end += 1
    body = src[start : end + 1]
    body = re.sub(r"//[^\n]*", "", body)
    body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
    body = re.sub(r"^(\s*)([A-Za-z_$][\w$]*)\s*:", r'\1"\2":', body, flags=re.M)
    body = re.sub(r",(\s*[}\]])", r"\1", body)
    return json.loads(body)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: gen_formats.py <path-to-ConvertX>")
    root = pathlib.Path(sys.argv[1])
    out = []
    for priority, meta in enumerate(CONVERTERS):
        source = root / "src" / "converters" / f"{meta['id']}.ts"
        props = extract_properties(source) if source.exists() else {"from": {}, "to": {}}
        entry = dict(meta)
        entry["priority"] = priority
        entry["from"] = props["from"]
        entry["to"] = props["to"]
        out.append(entry)

    target = pathlib.Path(__file__).resolve().parent.parent / "assets" / "formats.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps({"converters": out}, indent=1, sort_keys=False) + "\n")
    total_from = sum(len(v) for c in out for v in c["from"].values())
    total_to = sum(len(v) for c in out for v in c["to"].values())
    print(f"wrote {target} ({len(out)} converters, {total_from} inputs, {total_to} outputs)")


if __name__ == "__main__":
    main()
