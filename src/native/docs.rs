//! Document text extraction without LibreOffice: DOCX, ODT, EPUB, RTF and HTML.

use super::{failure, success};
use crate::convert::{ConvertRequest, ConvertResult};
use std::io::{Cursor, Read};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Heading(u8),
    Paragraph,
    Item,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
}

pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult {
    let bytes = match std::fs::read(&request.input) {
        Ok(bytes) => bytes,
        Err(err) => {
            return failure(
                format!("Cannot read {}: {err}", request.input.display()),
                started,
            );
        }
    };

    let blocks = match request.from.as_str() {
        "docx" | "docm" => archive_text(&bytes, "word/document.xml", parse_docx),
        "odt" | "ott" => archive_text(&bytes, "content.xml", parse_odt),
        "epub" => read_epub(&bytes),
        "rtf" | "rtfd" => Ok(parse_rtf(&String::from_utf8_lossy(&bytes))),
        "html" | "htm" | "xhtml" | "webarchive" => Ok(parse_html(&String::from_utf8_lossy(&bytes))),
        _ => Ok(plain_blocks(&String::from_utf8_lossy(&bytes))),
    };

    let blocks = match blocks {
        Ok(blocks) => blocks,
        Err(err) => return failure(err, started),
    };

    if blocks.iter().all(|block| block.text.trim().is_empty()) {
        return failure(
            format!(
                "No text found in {} — it may be image-only",
                request.input.display()
            ),
            started,
        );
    }

    let body = match request.to.as_str() {
        "markdown" => render_markdown(&blocks),
        "html" => render_html(&blocks),
        _ => render_text(&blocks),
    };

    if let Err(err) = std::fs::write(&request.output, body) {
        return failure(
            format!("Cannot write {}: {err}", request.output.display()),
            started,
        );
    }

    success(
        format!("Extracted {} block(s)", blocks.len()),
        &request.output,
        started,
    )
}

fn archive_text(
    bytes: &[u8],
    entry: &str,
    parse: fn(&str) -> Vec<Block>,
) -> Result<Vec<Block>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| format!("Not a valid archive: {err}"))?;
    let mut file = archive
        .by_name(entry)
        .map_err(|_| format!("Missing '{entry}' inside the document"))?;
    let mut xml = String::new();
    file.read_to_string(&mut xml)
        .map_err(|err| format!("Cannot read '{entry}': {err}"))?;
    Ok(parse(&xml))
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

fn parse_docx(xml: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = String::new();
    let mut kind = Kind::Paragraph;
    let mut in_text = false;
    let mut started = false;

    walk(xml, |event| match event {
        Event::Start { name, attrs, .. } => match name {
            "w:p" => {
                started = true;
                current.clear();
                kind = Kind::Paragraph;
            }
            "w:pStyle" => {
                if let Some(value) = attr(&attrs, "w:val")
                    && let Some(level) = heading_level(value)
                {
                    kind = Kind::Heading(level);
                }
            }
            "w:outlineLvl" => {
                if let Some(value) = attr(&attrs, "w:val").and_then(|v| v.parse::<u8>().ok()) {
                    kind = Kind::Heading((value + 1).min(6));
                }
            }
            "w:numPr" => {
                if kind == Kind::Paragraph {
                    kind = Kind::Item;
                }
            }
            "w:t" => in_text = true,
            "w:tab" => current.push('\t'),
            "w:br" | "w:cr" => current.push('\n'),
            _ => {}
        },
        Event::End { name } => match name {
            "w:t" => in_text = false,
            "w:p" => {
                if started {
                    push_block(&mut blocks, kind, &mut current);
                }
                started = false;
            }
            _ => {}
        },
        Event::Text(text) => {
            if in_text {
                current.push_str(&text);
            }
        }
    });

    if started {
        push_block(&mut blocks, kind, &mut current);
    }
    blocks
}

// ---------------------------------------------------------------------------
// ODT
// ---------------------------------------------------------------------------

fn parse_odt(xml: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = String::new();
    let mut kind = Kind::Paragraph;
    let mut inside = false;

    walk(xml, |event| match event {
        Event::Start { name, attrs, .. } => match name {
            "text:h" => {
                inside = true;
                current.clear();
                let level = attr(&attrs, "text:outline-level")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(1)
                    .clamp(1, 6);
                kind = Kind::Heading(level);
            }
            "text:p" => {
                inside = true;
                current.clear();
                if kind != Kind::Item || blocks.is_empty() {
                    kind = Kind::Paragraph;
                }
            }
            "text:list-item" => kind = Kind::Item,
            "text:s" => {
                let count = attr(&attrs, "text:c")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                if inside {
                    current.push_str(&" ".repeat(count.min(32)));
                }
            }
            "text:tab" => {
                if inside {
                    current.push('\t');
                }
            }
            "text:line-break" if inside => {
                current.push('\n');
            }
            _ => {}
        },
        Event::End { name } => {
            if matches!(name, "text:p" | "text:h") && inside {
                push_block(&mut blocks, kind, &mut current);
                inside = false;
                kind = Kind::Paragraph;
            }
        }
        Event::Text(text) => {
            if inside {
                current.push_str(&text);
            }
        }
    });

    blocks
}

// ---------------------------------------------------------------------------
// EPUB
// ---------------------------------------------------------------------------

fn read_epub(bytes: &[u8]) -> Result<Vec<Block>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| format!("Not a valid EPUB: {err}"))?;

    let mut chapters: Vec<String> = Vec::new();
    for index in 0..archive.len() {
        let Ok(mut file) = archive.by_index(index) else {
            continue;
        };
        let name = file.name().unwrap_or_default().to_string();
        let lower = name.to_lowercase();
        if lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm") {
            let mut text = String::new();
            if file.read_to_string(&mut text).is_ok() {
                chapters.push(text);
            }
        }
    }

    if chapters.is_empty() {
        return Err("The EPUB has no readable chapters".into());
    }

    let mut blocks = Vec::new();
    for chapter in chapters {
        blocks.extend(parse_html(&chapter));
    }
    Ok(blocks)
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

fn parse_html(html: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = String::new();
    let mut kind = Kind::Paragraph;
    let mut skip_depth = 0usize;

    walk(html, |event| match event {
        Event::Start { name, .. } => match name {
            "script" | "style" | "head" => skip_depth += 1,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut blocks, kind, &mut current);
                kind = Kind::Heading(name[1..].parse::<u8>().unwrap_or(1));
            }
            "p" | "div" | "section" | "article" | "blockquote" | "pre" | "td" | "th" => {
                flush(&mut blocks, kind, &mut current);
                kind = Kind::Paragraph;
            }
            "li" => {
                flush(&mut blocks, kind, &mut current);
                kind = Kind::Item;
            }
            "br" => current.push('\n'),
            _ => {}
        },
        Event::End { name } => match name {
            "script" | "style" | "head" => skip_depth = skip_depth.saturating_sub(1),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "li" | "blockquote" | "pre"
            | "td" | "th" => flush(&mut blocks, kind, &mut current),
            _ => {}
        },
        Event::Text(text) => {
            if skip_depth == 0 {
                current.push_str(&text);
            }
        }
    });
    flush(&mut blocks, kind, &mut current);
    blocks
}

// ---------------------------------------------------------------------------
// RTF
// ---------------------------------------------------------------------------

/// Minimal RTF reader: enough for `\par`, `\tab`, `\uN` and escaped characters.
fn parse_rtf(rtf: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = String::new();
    let mut chars = rtf.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => {}
            '}' => {}
            '\\' => {
                let mut word = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphabetic() {
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let mut number = String::new();
                let mut negative = false;
                if chars.peek() == Some(&'-') {
                    negative = true;
                    chars.next();
                }
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        number.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }

                match word.as_str() {
                    "par" | "line" | "sect" | "row" => {
                        flush(&mut blocks, Kind::Paragraph, &mut current);
                    }
                    "tab" => current.push('\t'),
                    "u" => {
                        if let Ok(code) = number.parse::<i32>() {
                            let code = if code < 0 { code + 65536 } else { code };
                            if let Some(value) = char::from_u32(code as u32) {
                                current.push(value);
                            }
                            // Skip the fallback ANSI character that follows.
                            if chars.peek() == Some(&'?') {
                                chars.next();
                            }
                        }
                    }
                    "bullet" => current.push('•'),
                    "emdash" => current.push('—'),
                    "endash" => current.push('–'),
                    "lquote" => current.push('‘'),
                    "rquote" => current.push('’'),
                    "ldblquote" => current.push('“'),
                    "rdblquote" => current.push('”'),
                    "" => {
                        if let Some(escaped) = chars.next() {
                            if escaped == '\'' {
                                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                                    current.push(byte as char);
                                }
                            } else if escaped == '*' {
                                // Ignorable group marker.
                            } else if escaped != '\n' && escaped != '\r' {
                                current.push(escaped);
                            }
                        }
                    }
                    _ => {
                        let _ = negative;
                    }
                }
            }
            '\r' | '\n' => {}
            other => current.push(other),
        }
    }
    flush(&mut blocks, Kind::Paragraph, &mut current);
    blocks
}

// ---------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------

fn plain_blocks(text: &str) -> Vec<Block> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(|block| Block {
            kind: Kind::Paragraph,
            text: block.replace("\r\n", "\n"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        if block.kind == Kind::Item {
            out.push_str("- ")
        }
        out.push_str(&block.text);
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

fn render_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block.kind {
            Kind::Heading(level) => {
                out.push_str(&"#".repeat(level as usize));
                out.push(' ');
                out.push_str(block.text.trim());
            }
            Kind::Item => {
                out.push_str("- ");
                out.push_str(block.text.trim());
            }
            Kind::Paragraph => out.push_str(block.text.trim()),
        }
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

fn render_html(blocks: &[Block]) -> String {
    let mut out = String::from(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n",
    );
    let mut in_list = false;
    for block in blocks {
        match block.kind {
            Kind::Item => {
                if !in_list {
                    out.push_str("<ul>\n");
                    in_list = true;
                }
                out.push_str(&format!("<li>{}</li>\n", escape_html(block.text.trim())));
                continue;
            }
            _ => {
                if in_list {
                    out.push_str("</ul>\n");
                    in_list = false;
                }
            }
        }
        match block.kind {
            Kind::Heading(level) => out.push_str(&format!(
                "<h{level}>{}</h{level}>\n",
                escape_html(block.text.trim())
            )),
            _ => out.push_str(&format!(
                "<p>{}</p>\n",
                escape_html(&block.text).replace('\n', "<br>\n")
            )),
        }
    }
    if in_list {
        out.push_str("</ul>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn flush(blocks: &mut Vec<Block>, kind: Kind, current: &mut String) {
    let text = current.trim().to_string();
    current.clear();
    if !text.is_empty() {
        blocks.push(Block { kind, text });
    }
}

fn push_block(blocks: &mut Vec<Block>, kind: Kind, current: &mut String) {
    let text = current.trim_end().to_string();
    current.clear();
    if !text.trim().is_empty() {
        blocks.push(Block { kind, text });
    }
}

fn heading_level(style: &str) -> Option<u8> {
    let lower = style.to_lowercase();
    if !lower.contains("heading") && !lower.starts_with("title") && !lower.contains("berschrift") {
        return None;
    }
    let digits: String = style.chars().filter(char::is_ascii_digit).collect();
    Some(digits.parse::<u8>().ok().unwrap_or(1).clamp(1, 6))
}

// ---------------------------------------------------------------------------
// Tiny XML/HTML scanner
// ---------------------------------------------------------------------------

enum Event<'a> {
    Start {
        name: &'a str,
        attrs: Vec<(&'a str, &'a str)>,
    },
    End {
        name: &'a str,
    },
    Text(String),
}

fn attr<'a>(attrs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| *value)
}

fn walk<'a>(input: &'a str, mut sink: impl FnMut(Event<'a>)) {
    let bytes = input.as_bytes();
    let mut index = 0usize;
    let mut text_start = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }

        if index > text_start {
            let text = decode_entities(&input[text_start..index]);
            if !text.is_empty() {
                sink(Event::Text(text));
            }
        }

        if input[index..].starts_with("<!--") {
            index = input[index..]
                .find("-->")
                .map(|end| index + end + 3)
                .unwrap_or(bytes.len());
            text_start = index;
            continue;
        }
        if input[index..].starts_with("<?") || input[index..].starts_with("<!") {
            index = input[index..]
                .find('>')
                .map(|end| index + end + 1)
                .unwrap_or(bytes.len());
            text_start = index;
            continue;
        }
        if !input[index..].starts_with('<') {
            index += 1;
            continue;
        }

        let Some(close) = find_tag_end(input, index) else {
            break;
        };
        let raw = &input[index + 1..close];
        index = close + 1;
        text_start = index;

        if let Some(name) = raw.strip_prefix('/') {
            sink(Event::End { name: name.trim() });
            continue;
        }

        let empty = raw.ends_with('/');
        let content = raw.trim_end_matches('/');
        let mut parts = content.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let mut attrs = Vec::new();
        if let Some(rest) = parts.next() {
            attrs = parse_attrs(rest);
        }
        sink(Event::Start { name, attrs });
        if empty {
            sink(Event::End { name });
        }
    }

    if text_start < bytes.len() {
        let text = decode_entities(&input[text_start..]);
        if !text.is_empty() {
            sink(Event::Text(text));
        }
    }
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut index = start + 1;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(q) if byte == q => quote = None,
            Some(_) => {}
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                } else if byte == b'>' {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

fn parse_attrs(rest: &str) -> Vec<(&str, &str)> {
    let mut attrs = Vec::new();
    let bytes = rest.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && !is_attr_name_byte(bytes[index]) {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len() && is_attr_name_byte(bytes[index]) {
            index += 1;
        }
        if name_start == index {
            break;
        }
        let name = &rest[name_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let quote = bytes[index];
        let value_start;
        let value_end;
        if quote == b'"' || quote == b'\'' {
            index += 1;
            value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            value_end = index;
            index += 1;
        } else {
            value_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            value_end = index;
        }
        attrs.push((name, &rest[value_start..value_end]));
    }
    attrs
}

fn is_attr_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.')
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(';').filter(|end| *end <= 12) else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| {
                    if let Some(hex) = number
                        .strip_prefix('x')
                        .or_else(|| number.strip_prefix('X'))
                    {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        number.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::ConvertOptions;

    #[test]
    fn extracts_docx_paragraphs() {
        let xml = r#"<w:document><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:r><w:t>world &amp; co</w:t></w:r></w:p>
            </w:body></w:document>"#;
        let blocks = parse_docx(xml);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, Kind::Heading(1));
        assert_eq!(blocks[0].text, "Report");
        assert_eq!(blocks[1].text, "Hello world & co");
    }

    #[test]
    fn extracts_html_blocks() {
        let blocks = parse_html(
            "<html><head><style>p{color:red}</style></head><body>\
             <h2>Title</h2><p>One <b>two</b></p><ul><li>A</li><li>B</li></ul></body></html>",
        );
        assert_eq!(blocks[0].kind, Kind::Heading(2));
        assert_eq!(blocks[0].text, "Title");
        assert_eq!(blocks[1].text, "One two");
        assert_eq!(blocks[2].kind, Kind::Item);
        assert_eq!(blocks[3].kind, Kind::Item);
    }

    #[test]
    fn extracts_rtf_text() {
        let blocks = parse_rtf(r"{\rtf1\ansi Hello\par World\'20again\par}");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "Hello");
        assert_eq!(blocks[1].text, "World again");
    }

    #[test]
    fn renders_markdown_and_writes_file() {
        let dir = std::env::temp_dir().join(format!("convertx-docs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("notes.html");
        std::fs::write(&input, "<h1>Hi</h1><p>Body</p>").unwrap();

        for target in ["txt", "markdown", "html"] {
            let request = ConvertRequest {
                job_id: 1,
                input: input.clone(),
                from: "html".into(),
                to: target.into(),
                converter_id: "native-docs".into(),
                output: dir.join(format!("notes.{target}")),
                options: ConvertOptions::default(),
            };
            let result = convert(&request, Instant::now());
            assert!(result.ok, "html -> {target}: {}", result.message);
            let text = std::fs::read_to_string(&request.output).unwrap();
            assert!(text.contains("Hi") && text.contains("Body"), "{text}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
