use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::domain::LibraryFileRecord;

pub(super) fn library_system_metadata(
    file: &LibraryFileRecord,
    folder_path: &str,
    section_key: &str,
    section_label: &str,
) -> Value {
    json!({
        "is_library_file": true,
        "library_file_id": file.id,
        "library_path": folder_path,
        "library_section_key": section_key,
        "library_section_label": section_label,
        "library_filename": file.filename,
        "library_media_type": file.media_type,
    })
}

pub(super) fn compose_library_metadata(
    section_metadata: &Value,
    file_metadata: &Value,
    system_metadata: Value,
) -> Result<Value> {
    let Some(system_object) = system_metadata.as_object() else {
        return Err(anyhow!("system library metadata must be an object"));
    };
    let mut merged = match section_metadata {
        Value::Null => serde_json::Map::new(),
        Value::Object(map) => map.clone(),
        _ => return Err(anyhow!("metadata_json must be an object")),
    };
    let file_object = file_metadata
        .as_object()
        .ok_or_else(|| anyhow!("file metadata_json must be an object"))?;
    for (key, value) in file_object {
        merged.insert(key.clone(), value.clone());
    }
    merged.remove("record_hash");
    for (key, value) in system_object {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::compose_library_metadata;

    #[test]
    fn file_metadata_overrides_section_and_system_fields_cannot_be_forged() {
        let merged = compose_library_metadata(
            &json!({"score": 1, "library_file_id": "section", "section_only": true}),
            &json!({"score": 10, "library_file_id": "caller", "record_hash": "fake"}),
            json!({"library_file_id": "system", "is_library_file": true}),
        )
        .expect("metadata should compose");

        assert_eq!(merged["score"], json!(10));
        assert_eq!(merged["library_file_id"], json!("system"));
        assert!(merged.get("record_hash").is_none());
    }
}
