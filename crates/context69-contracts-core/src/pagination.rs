use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub const PAGE_MIN: u32 = 1;
pub const PAGE_MAX: u32 = 10_000;
pub const PAGE_SIZE_MIN: u32 = 1;
pub const PAGE_SIZE_MAX: u32 = 100;
pub const CURSOR_LIMIT_MIN: u32 = 1;
pub const CURSOR_LIMIT_MAX: u32 = 100;

pub fn default_page() -> u32 {
    1
}

pub fn default_page_size() -> u32 {
    50
}

pub fn default_limit() -> u32 {
    50
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

impl SortDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

impl std::fmt::Display for SortDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SortDirection {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "asc" => Ok(Self::Asc),
            "desc" => Ok(Self::Desc),
            other => Err(anyhow::anyhow!("unsupported sort direction: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct OffsetPageQuery {
    #[serde(default = "default_page")]
    #[param(minimum = 1, maximum = 10_000)]
    #[schema(minimum = 1, maximum = 10_000)]
    #[schemars(range(min = 1, max = 10_000))]
    pub page: u32,
    #[serde(default = "default_page_size")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub page_size: u32,
}

impl Default for OffsetPageQuery {
    fn default() -> Self {
        Self {
            page: default_page(),
            page_size: default_page_size(),
        }
    }
}

impl OffsetPageQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(PAGE_MIN..=PAGE_MAX).contains(&self.page) {
            return Err(anyhow::anyhow!("page must be between 1 and 10000"));
        }
        if !(PAGE_SIZE_MIN..=PAGE_SIZE_MAX).contains(&self.page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        Ok(())
    }

    pub fn offset(&self) -> anyhow::Result<i64> {
        self.validate()?;
        i64::from(self.page - 1)
            .checked_mul(i64::from(self.page_size))
            .ok_or_else(|| anyhow::anyhow!("page offset is too large"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct CursorPageQuery {
    #[serde(default = "default_limit")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl Default for CursorPageQuery {
    fn default() -> Self {
        Self {
            limit: default_limit(),
            cursor: None,
        }
    }
}

impl CursorPageQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(CURSOR_LIMIT_MIN..=CURSOR_LIMIT_MAX).contains(&self.limit) {
            return Err(anyhow::anyhow!("limit must be between 1 and 100"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct OffsetPagination {
    #[schema(minimum = 1, maximum = 10_000)]
    #[schemars(range(min = 1, max = 10_000))]
    pub page: u32,
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub page_size: u32,
    pub total: u64,
    pub total_pages: u32,
}

impl OffsetPagination {
    pub fn try_new(page: u32, page_size: u32, total: u64) -> anyhow::Result<Self> {
        if !(PAGE_MIN..=PAGE_MAX).contains(&page) {
            return Err(anyhow::anyhow!("page must be between 1 and 10000"));
        }
        if !(PAGE_SIZE_MIN..=PAGE_SIZE_MAX).contains(&page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        let total_pages = if total == 0 {
            0
        } else {
            u32::try_from(total.div_ceil(u64::from(page_size)))?
        };
        Ok(Self {
            page,
            page_size,
            total,
            total_pages,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CursorPagination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl CursorPagination {
    pub fn new(next_cursor: Option<String>, has_more: bool) -> Self {
        Self {
            next_cursor,
            has_more,
        }
    }

    pub fn terminal() -> Self {
        Self {
            next_cursor: None,
            has_more: false,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !self.has_more && self.next_cursor.is_none()
    }

    pub fn validate_continuation(&self) -> anyhow::Result<()> {
        if self.has_more && self.next_cursor.is_none() {
            return Err(anyhow::anyhow!(
                "has_more=true requires a next_cursor continuation token"
            ));
        }
        Ok(())
    }
}
