use std::io::{self, Write};

use crate::docling::MAX_DOCLING_OUTPUT_BYTES;

use super::*;

pub(super) fn ensure_json_output_size(value: &Value) -> Result<usize> {
    json_output_size(value, MAX_DOCLING_OUTPUT_BYTES)
}

fn json_output_size(value: &Value, limit: usize) -> Result<usize> {
    let mut writer = LimitedWriter { written: 0, limit };
    serde_json::to_writer(&mut writer, value).context("Docling JSON output is too large")?;
    Ok(writer.written)
}

pub(super) fn extract_json_text(value: &Value) -> Option<String> {
    let body = value.get("body")?;
    let groups = value.get("groups")?.as_array()?;
    let tables = value
        .get("tables")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let children = body.get("children")?.as_array()?;
    let mut parts = Vec::new();
    for child in children {
        if let Some(reference) = child.get("$ref").and_then(Value::as_str)
            && let Some(index) = reference
                .strip_prefix("#/groups/")
                .and_then(|s| s.parse::<usize>().ok())
            && let Some(group) = groups.get(index)
            && let Some(text) = group_text(group, tables)
            && !text.is_empty()
        {
            parts.push(text);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(super) fn extract_xlsx_sections(filename: &str, value: &Value) -> Result<Vec<IngestSection>> {
    let groups = value
        .get("groups")
        .and_then(Value::as_array)
        .context("xlsx docling json missing groups")?;
    let tables = value
        .get("tables")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut sections = Vec::new();

    for (index, group) in groups.iter().enumerate() {
        let name = group.get("name").and_then(Value::as_str).unwrap_or("sheet");
        let label = name.strip_prefix("sheet: ").unwrap_or(name).to_string();
        let body = group_text(group, tables).unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        sections.push(IngestSection {
            section_key: format!("sheet-{index}"),
            section_label: label.clone(),
            title: format!("{filename} / {label}"),
            summary: None,
            body_text: normalize_body(&body),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: serde_json::json!({}),
        });
    }

    Ok(sections)
}

fn group_text(group: &Value, tables: &[Value]) -> Option<String> {
    let children = group.get("children")?.as_array()?;
    let mut parts = Vec::new();

    for child in children {
        let reference = child.get("$ref").and_then(Value::as_str)?;
        if let Some(index) = reference
            .strip_prefix("#/tables/")
            .and_then(|value| value.parse::<usize>().ok())
            && let Some(table) = tables.get(index)
        {
            let text = table_to_text(table);
            if !text.is_empty() {
                parts.push(text);
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn table_to_text(table: &Value) -> String {
    let rows = table
        .get("data")
        .and_then(|value| value.get("grid"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut lines = Vec::new();
    for row in rows {
        let cols = row.as_array().cloned().unwrap_or_default();
        let mut values = Vec::new();
        let mut last = String::new();
        for col in cols {
            let text = normalize_whitespace(col.get("text").and_then(Value::as_str).unwrap_or(""));
            if text.is_empty() || text == last {
                continue;
            }
            last = text.clone();
            values.push(text);
        }
        if !values.is_empty() {
            lines.push(values.join(" | "));
        }
    }
    lines.join("\n")
}

struct LimitedWriter {
    written: usize,
    limit: usize,
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.written.saturating_add(bytes.len());
        if next > self.limit {
            return Err(io::Error::other(format!(
                "Docling output exceeds {} bytes",
                self.limit
            )));
        }
        self.written = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::json_output_size;

    #[test]
    fn rejects_json_output_before_copying_past_limit() {
        let value = json!({"body": "123456"});

        assert!(json_output_size(&value, 8).is_err());
        assert!(json_output_size(&value, 64).is_ok());
    }
}
