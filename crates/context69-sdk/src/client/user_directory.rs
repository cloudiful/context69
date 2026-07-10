use context69_contracts::UserDirectoryEntryResponse;
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct UserDirectoryApi<'a> {
    client: &'a Context69Client,
}

impl<'a> UserDirectoryApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserDirectoryEntryResponse>, Error> {
        let mut url = self.client.url("/v1/user-directory")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("query", query);
            pairs.append_pair("limit", &limit.to_string());
        }
        self.client
            .execute_json(self.client.authorized_url_request(Method::GET, url).await?)
            .await
    }
}
