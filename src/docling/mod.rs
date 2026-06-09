use std::time::Duration;

mod client;
mod vlm;

pub use client::{DoclingClient, DoclingParsedDocument, DoclingRequest};
use serde::{Deserialize, Serialize};

use crate::serde_helpers;

pub const VALID_OCR_ENGINES: &[&str] = &[
    "auto",
    "easyocr",
    "kserve_v2_ocr",
    "ocrmac",
    "rapidocr",
    "tesserocr",
    "tesseract",
];
pub const VALID_IMAGE_EXPORT_MODES: &[&str] = &["placeholder", "embedded", "referenced"];
pub const VALID_PDF_BACKENDS: &[&str] = &[
    "pypdfium2",
    "docling_parse",
    "dlparse_v1",
    "dlparse_v2",
    "dlparse_v4",
];
pub const DEFAULT_DOCLING_BASE_URL: &str = "http://127.0.0.1:5001";
pub const DEFAULT_DOCLING_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_DOCLING_POLL_INTERVAL_SECS: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConnectionConfig {
    pub base_url: String,
    #[serde(rename = "timeout_secs", with = "serde_helpers::seconds")]
    pub timeout: Duration,
    #[serde(rename = "poll_interval_secs", with = "serde_helpers::seconds")]
    pub poll_interval: Duration,
}

impl Default for DoclingConnectionConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DOCLING_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS),
            poll_interval: Duration::from_secs(DEFAULT_DOCLING_POLL_INTERVAL_SECS),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConversionConfig {
    pub pdf_backend: Option<String>,
    pub images_scale: Option<f64>,
    pub image_export_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingOcrConfig {
    pub do_ocr: bool,
    pub force_ocr: bool,
    pub ocr_engine: Option<String>,
    pub ocr_lang: Vec<String>,
}

impl Default for DoclingOcrConfig {
    fn default() -> Self {
        Self {
            do_ocr: true,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingEnrichmentConfig {
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub do_picture_description: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingVlmConfig {
    pub openai_base_url: Option<String>,
    pub api_key: Option<String>,
    pub vlm_pipeline_model: Option<String>,
    pub picture_description_model: Option<String>,
    pub code_formula_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConfig {
    #[serde(flatten)]
    pub connection: DoclingConnectionConfig,
    pub conversion: DoclingConversionConfig,
    pub ocr: DoclingOcrConfig,
    pub enrichment: DoclingEnrichmentConfig,
    pub vlm: DoclingVlmConfig,
}

impl Default for DoclingConfig {
    fn default() -> Self {
        Self {
            connection: DoclingConnectionConfig::default(),
            conversion: DoclingConversionConfig::default(),
            ocr: DoclingOcrConfig::default(),
            enrichment: DoclingEnrichmentConfig::default(),
            vlm: DoclingVlmConfig::default(),
        }
    }
}

impl DoclingConfig {
    pub fn enrichment_enabled(&self) -> bool {
        self.enrichment.do_code_enrichment
            || self.enrichment.do_formula_enrichment
            || self.enrichment.do_picture_description
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoclingInputKind {
    Pdf,
    Docx,
    Xlsx,
}

#[derive(Debug, Clone, Copy)]
pub enum DoclingOutput {
    Text,
    Json,
}

impl DoclingOutput {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

fn bool_as_string(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;

    use super::{
        DoclingClient, DoclingConfig, DoclingConnectionConfig, DoclingConversionConfig,
        DoclingEnrichmentConfig, DoclingInputKind, DoclingOcrConfig, DoclingOutput, DoclingRequest,
        DoclingVlmConfig,
    };

    fn sample_config() -> DoclingConfig {
        DoclingConfig {
            connection: DoclingConnectionConfig {
                base_url: "http://localhost:5001".to_string(),
                timeout: Duration::from_secs(120),
                poll_interval: Duration::from_secs(2),
            },
            conversion: DoclingConversionConfig {
                pdf_backend: Some("dlparse_v2".to_string()),
                images_scale: Some(2.0),
                image_export_mode: Some("placeholder".to_string()),
            },
            ocr: DoclingOcrConfig {
                do_ocr: true,
                force_ocr: false,
                ocr_engine: Some("rapidocr".to_string()),
                ocr_lang: vec!["en".to_string(), "zh".to_string()],
            },
            enrichment: DoclingEnrichmentConfig {
                do_code_enrichment: true,
                do_formula_enrichment: true,
                do_picture_description: true,
            },
            vlm: DoclingVlmConfig {
                openai_base_url: Some("https://example.com/v1".to_string()),
                api_key: Some("secret".to_string()),
                vlm_pipeline_model: Some("vlm".to_string()),
                picture_description_model: Some("pic".to_string()),
                code_formula_model: Some("code".to_string()),
            },
        }
    }

    #[test]
    fn pdf_form_includes_ocr_conversion_and_vlm_fields() {
        let client = DoclingClient::new(sample_config()).expect("client");
        let form = client
            .build_form(&DoclingRequest {
                filename: "sample.pdf".to_string(),
                media_type: "application/pdf".to_string(),
                bytes: Bytes::from_static(b"pdf"),
                from_format: "pdf",
                outputs: vec![DoclingOutput::Text],
                page_range: Some((1, 2)),
                kind: DoclingInputKind::Pdf,
            })
            .expect("form");
        let debug = format!("{form:?}");

        assert!(debug.contains("ocr_engine"));
        assert!(debug.contains("ocr_lang"));
        assert!(debug.contains("pdf_backend"));
        assert!(debug.contains("images_scale"));
        assert!(debug.contains("image_export_mode"));
        assert!(debug.contains("vlm_pipeline_custom_config"));
        assert!(debug.contains("picture_description_custom_config"));
        assert!(debug.contains("code_formula_custom_config"));
        assert!(debug.contains("page_range"));
    }

    #[test]
    fn docx_form_skips_pdf_only_fields() {
        let client = DoclingClient::new(sample_config()).expect("client");
        let form = client
            .build_form(&DoclingRequest {
                filename: "sample.docx".to_string(),
                media_type:
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        .to_string(),
                bytes: Bytes::from_static(b"docx"),
                from_format: "docx",
                outputs: vec![DoclingOutput::Text, DoclingOutput::Json],
                page_range: None,
                kind: DoclingInputKind::Docx,
            })
            .expect("form");
        let debug = format!("{form:?}");

        assert!(!debug.contains("pdf_backend"));
        assert!(!debug.contains("ocr_engine"));
        assert!(!debug.contains("ocr_lang"));
        assert!(debug.contains("images_scale"));
        assert!(debug.contains("image_export_mode"));
        assert!(debug.contains("vlm_pipeline_custom_config"));
    }

    #[test]
    fn xlsx_form_remains_minimal() {
        let client = DoclingClient::new(sample_config()).expect("client");
        let form = client
            .build_form(&DoclingRequest {
                filename: "sample.xlsx".to_string(),
                media_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    .to_string(),
                bytes: Bytes::from_static(b"xlsx"),
                from_format: "xlsx",
                outputs: vec![DoclingOutput::Json],
                page_range: None,
                kind: DoclingInputKind::Xlsx,
            })
            .expect("form");
        let debug = format!("{form:?}");

        assert!(!debug.contains("ocr_engine"));
        assert!(!debug.contains("vlm_pipeline_custom_config"));
        assert!(!debug.contains("images_scale"));
    }
}
