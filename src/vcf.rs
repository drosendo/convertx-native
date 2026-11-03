use crate::convert::{ConvertRequest, ConvertResult};
use std::time::Instant;

/// Built-in VCF -> CSV conversion. Upstream implements this in TypeScript as well;
/// keeping it native means contact exports work without any external package.
pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult {
    if request.from != "vcf" || request.to != "csv" {
        return ConvertResult {
            ok: false,
            message: format!(
                "The built-in converter only supports vcf → csv (asked for {} → {})",
                request.from, request.to
            ),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    let bytes = match std::fs::read(&request.input) {
        Ok(bytes) => bytes,
        Err(err) => {
            return ConvertResult {
                ok: false,
                message: format!("Could not read vCard file: {err}"),
                duration: started.elapsed(),
                ..Default::default()
            };
        }
    };

    let text = String::from_utf8_lossy(&bytes);
    let cards = parse_vcards(&text);
    if cards.is_empty() {
        return ConvertResult {
            ok: false,
            message: "No VCARD entries found in the file".into(),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    let csv = to_csv(&cards);
    if let Err(err) = std::fs::write(&request.output, csv) {
        return ConvertResult {
            ok: false,
            message: format!("Could not write CSV: {err}"),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    ConvertResult {
        ok: true,
        message: format!("Converted {} contact(s)", cards.len()),
        output: Some(request.output.clone()),
        log: String::new(),
        duration: started.elapsed(),
        converter: Some(request.converter_id.clone()),
        attempts: Vec::new(),
    }
}

type Contact = Vec<(String, String)>;

fn parse_vcards(data: &str) -> Vec<Contact> {
    let unfolded = unfold(data);
    let mut contacts = Vec::new();

    for block in unfolded.split("BEGIN:VCARD").skip(1) {
        let card = block.split("END:VCARD").next().unwrap_or("");
        let mut contact: Contact = Vec::new();

        for line in card.lines() {
            let line = line.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_uppercase();
            let value = unescape(value.trim());

            if key == "FN" {
                push(&mut contact, "Full Name", &value);
            } else if key == "N" {
                let parts: Vec<&str> = value.split(';').collect();
                push(&mut contact, "Last Name", parts.first().copied().unwrap_or(""));
                push(&mut contact, "First Name", parts.get(1).copied().unwrap_or(""));
            } else if key.starts_with("TEL") && !has(&contact, "Phone") {
                push(&mut contact, "Phone", &value);
            } else if key.starts_with("EMAIL") && !has(&contact, "Email") {
                push(&mut contact, "Email", &value);
            } else if key == "ORG" {
                let org = value.split(';').next().unwrap_or("");
                push(&mut contact, "Organization", org);
            }
        }

        if !contact.is_empty() {
            contacts.push(contact);
        }
    }

    contacts
}

/// vCard folds long lines by starting continuations with a space or tab.
fn unfold(data: &str) -> String {
    let mut out = String::with_capacity(data.len());
    for line in data.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.starts_with(' ') || line.starts_with('\t') {
            out.push_str(line.trim_start());
        } else {
            out.push('\n');
            out.push_str(line);
        }
    }
    out
}

fn unescape(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

fn has(contact: &Contact, key: &str) -> bool {
    contact.iter().any(|(k, _)| k == key)
}

fn push(contact: &mut Contact, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    if let Some(entry) = contact.iter_mut().find(|(k, _)| k == key) {
        if entry.1.trim().is_empty() {
            entry.1 = value.to_string();
        }
        return;
    }
    contact.push((key.to_string(), value.to_string()));
}

fn to_csv(contacts: &[Contact]) -> String {
    let mut headers: Vec<String> = Vec::new();
    for contact in contacts {
        for (key, _) in contact {
            if !headers.iter().any(|h| h == key) {
                headers.push(key.clone());
            }
        }
    }

    let escape = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
    let mut lines = Vec::with_capacity(contacts.len() + 1);
    lines.push(
        headers
            .iter()
            .map(|h| escape(h))
            .collect::<Vec<_>>()
            .join(","),
    );
    for contact in contacts {
        let row = headers
            .iter()
            .map(|header| {
                let value = contact
                    .iter()
                    .find(|(k, _)| k == header)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                escape(value)
            })
            .collect::<Vec<_>>()
            .join(",");
        lines.push(row);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_flattens_contacts() {
        let data = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada;;;\r\nTEL;TYPE=CELL:+44 20 1234\r\nEMAIL:ada@example.com\r\nORG:Analytical Engines;R&D\r\nEND:VCARD\r\n";
        let contacts = parse_vcards(data);
        assert_eq!(contacts.len(), 1);
        let csv = to_csv(&contacts);
        assert!(csv.contains("\"Full Name\""));
        assert!(csv.contains("\"Ada Lovelace\""));
        assert!(csv.contains("+44 20 1234"));
        assert!(csv.contains("\"Analytical Engines\""));
    }

    #[test]
    fn quotes_embedded_commas() {
        let contacts = vec![vec![("Full Name".to_string(), "Doe, Jane".to_string())]];
        assert_eq!(to_csv(&contacts), "\"Full Name\"\n\"Doe, Jane\"");
    }
}
