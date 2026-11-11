//! Response shapes of the Forgejo/Gitea Git REST API.
//!
//! Only the fields acquisition reads are modelled; unknown fields are ignored,
//! and every identifier is validated by the caller before it is used in another
//! request. `type` is renamed because it is a Rust keyword.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryResponse {
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reference {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub object: GitObject,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitObject {
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitTreeResponse {
    pub sha: String,
    #[serde(deserialize_with = "deserialize_null_default")]
    pub tree: Vec<GitEntry>,
    pub truncated: bool,
    #[serde(default)]
    pub page: i64,
    pub total_count: i64,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitEntry {
    pub path: String,
    pub sha: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitBlobResponse {
    pub content: String,
    pub encoding: String,
    pub sha: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnnotatedTag {
    pub sha: String,
    pub target: GitObject,
}
