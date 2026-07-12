use anyhow::{Result, anyhow};

const MAX_SEGMENT_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationSegment {
    pub id: String,
    pub text: String,
    pub suffix: String,
    pub translatable: bool,
}

#[derive(Debug, Clone)]
pub struct SegmentedDocument {
    pub title: TranslationSegment,
    pub summary: Option<TranslationSegment>,
    pub body: Vec<TranslationSegment>,
}

pub fn segment_document(title: &str, summary: Option<&str>, body: &str) -> SegmentedDocument {
    SegmentedDocument {
        title: TranslationSegment {
            id: "title".to_string(),
            text: title.to_string(),
            suffix: String::new(),
            translatable: true,
        },
        summary: summary.map(|value| TranslationSegment {
            id: "summary".to_string(),
            text: value.to_string(),
            suffix: String::new(),
            translatable: true,
        }),
        body: segment_body(body),
    }
}

pub fn translated_document(
    source: &SegmentedDocument,
    translations: &std::collections::HashMap<String, String>,
) -> Result<(String, Option<String>, String)> {
    let title = translated_value(&source.title, translations)?;
    let summary = source
        .summary
        .as_ref()
        .map(|segment| translated_value(segment, translations))
        .transpose()?;
    let mut body = String::new();
    for segment in &source.body {
        body.push_str(&translated_value(segment, translations)?);
        body.push_str(&segment.suffix);
    }
    Ok((title, summary, body))
}

pub fn translatable_segments(document: &SegmentedDocument) -> Vec<TranslationSegment> {
    std::iter::once(&document.title)
        .chain(document.summary.iter())
        .chain(document.body.iter())
        .filter(|segment| segment.translatable && !segment.text.trim().is_empty())
        .cloned()
        .collect()
}

fn translated_value(
    segment: &TranslationSegment,
    translations: &std::collections::HashMap<String, String>,
) -> Result<String> {
    if !segment.translatable || segment.text.trim().is_empty() {
        return Ok(segment.text.clone());
    }
    translations
        .get(&segment.id)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| anyhow!("provider omitted translation segment {}", segment.id))
}

fn segment_body(body: &str) -> Vec<TranslationSegment> {
    let mut segments = Vec::new();
    let mut fenced = false;
    let mut pending = String::new();
    let mut pending_suffix = "";
    for line in body.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let suffix = if line.ends_with('\n') { "\n" } else { "" };
        let is_fence = content.trim_start().starts_with("```");
        if is_fence || fenced {
            flush_paragraph(&mut segments, &mut pending, pending_suffix);
            push_split(&mut segments, content, suffix, false);
            if is_fence {
                fenced = !fenced;
            }
            continue;
        }
        if content.trim().is_empty() {
            flush_paragraph(&mut segments, &mut pending, pending_suffix);
            push_split(&mut segments, content, suffix, false);
            continue;
        }
        if !pending.is_empty() {
            pending.push('\n');
        }
        pending.push_str(content);
        pending_suffix = suffix;
        if pending.chars().count() >= MAX_SEGMENT_CHARS {
            flush_paragraph(&mut segments, &mut pending, pending_suffix);
        }
    }
    flush_paragraph(&mut segments, &mut pending, pending_suffix);
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = format!("body:{index:06}");
    }
    segments
}

fn flush_paragraph(segments: &mut Vec<TranslationSegment>, pending: &mut String, suffix: &str) {
    if pending.is_empty() {
        return;
    }
    let value = std::mem::take(pending);
    push_split(segments, &value, suffix, true);
}

fn push_split(
    segments: &mut Vec<TranslationSegment>,
    value: &str,
    suffix: &str,
    translatable: bool,
) {
    if value.chars().count() <= MAX_SEGMENT_CHARS {
        segments.push(TranslationSegment {
            id: String::new(),
            text: value.to_string(),
            suffix: suffix.to_string(),
            translatable,
        });
        return;
    }
    let chars = value.chars().collect::<Vec<_>>();
    for (index, chunk) in chars.chunks(MAX_SEGMENT_CHARS).enumerate() {
        segments.push(TranslationSegment {
            id: String::new(),
            text: chunk.iter().collect(),
            suffix: if index + 1 == chars.chunks(MAX_SEGMENT_CHARS).len() {
                suffix.to_string()
            } else {
                String::new()
            },
            translatable,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_code_and_unicode_without_truncation() {
        let body = format!("Intro\n\n```rust\nlet x = 1;\n```\n{}", "中".repeat(8_100));
        let segmented = segment_document("Title", None, &body);
        let mut translated = std::collections::HashMap::new();
        translated.insert("title".to_string(), "标题".to_string());
        for segment in translatable_segments(&segmented) {
            translated.insert(segment.id, segment.text);
        }
        let (_, _, rebuilt) = translated_document(&segmented, &translated).unwrap();
        assert!(rebuilt.contains("let x = 1;"));
        assert!(rebuilt.chars().filter(|value| *value == '中').count() == 8_100);
    }

    #[test]
    fn preserves_exact_line_endings() {
        for body in ["one", "one\n", "one\ntwo", "one\ntwo\n", "one\n\ntwo"] {
            let segmented = segment_document("Title", None, body);
            let translations = translatable_segments(&segmented)
                .into_iter()
                .map(|segment| (segment.id, segment.text))
                .chain(std::iter::once(("title".to_string(), "Title".to_string())))
                .collect();
            let (_, _, rebuilt) = translated_document(&segmented, &translations).unwrap();
            assert_eq!(rebuilt, body);
        }
    }
}
