//! Spreadsheet reading through `calamine`, plus a small CSV/TSV reader so plain
//! delimited files work even where calamine has no parser.

use super::{failure, success};
use crate::convert::{ConvertRequest, ConvertResult};
use calamine::{Data, Reader};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Empty,
    Number(f64),
    Text(String),
}

pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult {
    let mut note = String::new();

    let rows: Vec<Vec<Cell>> = match request.from.as_str() {
        "csv" | "tsv" => {
            let bytes = match std::fs::read(&request.input) {
                Ok(bytes) => bytes,
                Err(err) => {
                    return failure(
                        format!("Cannot read {}: {err}", request.input.display()),
                        started,
                    );
                }
            };
            let delimiter = if request.from == "tsv" { b'\t' } else { b',' };
            read_delimited(&bytes, delimiter)
        }
        _ => match read_workbook(&request.input) {
            Ok((rows, sheet, count)) => {
                note = if count > 1 {
                    format!("Converted sheet '{sheet}' ({count} sheets in the workbook)")
                } else {
                    format!("Converted sheet '{sheet}'")
                };
                rows
            }
            Err(err) => return failure(err, started),
        },
    };

    if rows.is_empty() {
        return failure("The spreadsheet is empty", started);
    }

    let body = match request.to.as_str() {
        "tsv" => to_separated(&rows, '\t'),
        "json" => to_json(&rows),
        _ => to_separated(&rows, ','),
    };

    if let Err(err) = std::fs::write(&request.output, body) {
        return failure(
            format!("Cannot write {}: {err}", request.output.display()),
            started,
        );
    }

    if note.is_empty() {
        note = format!("Converted {} row(s)", rows.len());
    }
    success(note, &request.output, started)
}

fn read_workbook(path: &std::path::Path) -> Result<(Vec<Vec<Cell>>, String, usize), String> {
    let mut workbook = calamine::open_workbook_auto(path)
        .map_err(|err| format!("Cannot open {}: {err}", path.display()))?;

    let sheets = workbook.sheet_names().to_vec();
    let Some(first) = sheets.first().cloned() else {
        return Err("The workbook has no sheets".into());
    };
    let range = workbook
        .worksheet_range(&first)
        .map_err(|err| format!("Cannot read sheet '{first}': {err}"))?;

    let rows = range
        .rows()
        .map(|row| row.iter().map(cell_value).collect())
        .collect();
    Ok((rows, first, sheets.len()))
}

fn cell_value(cell: &Data) -> Cell {
    match cell {
        Data::Empty => Cell::Empty,
        Data::Int(value) => Cell::Number(*value as f64),
        Data::Float(value) => Cell::Number(*value),
        Data::Bool(value) => Cell::Text(value.to_string()),
        Data::String(value) => Cell::Text(value.clone()),
        Data::DateTime(value) => Cell::Text(value.to_string()),
        Data::DateTimeIso(value) | Data::DurationIso(value) => Cell::Text(value.clone()),
        Data::Error(value) => Cell::Text(format!("#{value:?}")),
    }
}

/// RFC 4180 style reader: quoted fields, doubled quotes and CRLF line endings.
fn read_delimited(bytes: &[u8], delimiter: u8) -> Vec<Vec<Cell>> {
    let text = String::from_utf8_lossy(bytes);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    fn push_field(row: &mut Vec<Cell>, field: &mut String) {
        let value = field.trim_end_matches('\r').to_string();
        row.push(classify(&value));
        field.clear();
    }

    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => quoted = true,
            '\n' => {
                push_field(&mut row, &mut field);
                rows.push(std::mem::take(&mut row));
            }
            _ if ch as u32 == delimiter as u32 => push_field(&mut row, &mut field),
            _ => field.push(ch),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        push_field(&mut row, &mut field);
        rows.push(row);
    }
    rows
}

fn classify(value: &str) -> Cell {
    if value.is_empty() {
        return Cell::Empty;
    }
    let trimmed = value.trim();
    if let Ok(number) = trimmed.parse::<f64>() {
        return Cell::Number(number);
    }
    Cell::Text(value.to_string())
}

fn text_of(cell: &Cell) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Number(value) => {
            if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        Cell::Text(value) => value.clone(),
    }
}

fn to_separated(rows: &[Vec<Cell>], separator: char) -> String {
    let mut out = String::new();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                out.push(separator);
            }
            out.push_str(&quote(&text_of(cell), separator));
        }
        out.push('\n');
    }
    out
}

fn to_json(rows: &[Vec<Cell>]) -> String {
    let mut encoded: Vec<String> = Vec::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| match cell {
                Cell::Empty => "null".to_string(),
                Cell::Number(value) => text_of(&Cell::Number(*value)),
                Cell::Text(value) => {
                    let escaped = value
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r")
                        .replace('\t', "\\t");
                    format!("\"{escaped}\"")
                }
            })
            .collect();
        encoded.push(format!("[{}]", cells.join(",")));
    }
    format!("[\n  {}\n]\n", encoded.join(",\n  "))
}

fn quote(value: &str, separator: char) -> String {
    let needs_quotes = value.contains(separator)
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r');
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::ConvertOptions;

    #[test]
    fn converts_csv_to_tsv_and_json() {
        let dir = std::env::temp_dir().join(format!("convertx-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("data.csv");
        std::fs::write(&input, "name,qty\n\"Widget, large\",12\nGadget,7\n").unwrap();

        let tsv = ConvertRequest {
            job_id: 1,
            input: input.clone(),
            from: "csv".into(),
            to: "tsv".into(),
            converter_id: "native-sheets".into(),
            output: dir.join("data.tsv"),
            options: ConvertOptions::default(),
        };
        let result = convert(&tsv, Instant::now());
        assert!(result.ok, "{}", result.message);
        let text = std::fs::read_to_string(&tsv.output).unwrap();
        assert!(text.starts_with("name\tqty"), "{text}");
        assert!(text.contains("Widget, large\t12"), "{text}");

        let json = ConvertRequest {
            job_id: 2,
            input,
            from: "csv".into(),
            to: "json".into(),
            converter_id: "native-sheets".into(),
            output: dir.join("data.json"),
            options: ConvertOptions::default(),
        };
        let result = convert(&json, Instant::now());
        assert!(result.ok, "{}", result.message);
        let text = std::fs::read_to_string(&json.output).unwrap();
        assert!(text.contains("[\"name\",\"qty\"]"), "{text}");
        assert!(text.contains("[\"Widget, large\",12]"), "{text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_quoted_fields_and_crlf() {
        let rows = read_delimited(b"a,\"b,\"\"c\"\"\",d\r\ne,f,g\r\n", b',');
        assert_eq!(rows.len(), 2);
        assert_eq!(text_of(&rows[0][1]), "b,\"c\"");
        assert_eq!(text_of(&rows[1][2]), "g");
    }

    #[test]
    fn numbers_stay_numbers() {
        assert_eq!(classify("12"), Cell::Number(12.0));
        assert_eq!(classify("12.5"), Cell::Number(12.5));
        assert_eq!(classify("abc"), Cell::Text("abc".into()));
        assert_eq!(classify(""), Cell::Empty);
    }
}
