mod namespace;
mod search;
mod settings;

pub use namespace::{NamespaceApiAdapter, UserDirectoryApiAdapter};
pub(crate) use namespace::{group_page_response, page_request};
pub use search::SearchApiAdapter;
pub use settings::SettingsApiAdapter;
