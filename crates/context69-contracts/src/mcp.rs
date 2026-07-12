use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentArgs {
    pub document_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}
