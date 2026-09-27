use anyhow::Result;

use context69_contracts::{DocumentQueryRequest, DocumentSortField};

use crate::{db::StoredMetadataIndex, domain_errors::DomainError};

pub(super) fn validate_query(request: &DocumentQueryRequest) -> Result<()> {
    if request.limit == 0 || request.limit > 200 {
        return Err(DomainError::invalid_argument("limit must be between 1 and 200").into());
    }
    if request.sort.len() > 3 {
        return Err(DomainError::invalid_argument("sort supports at most 3 fields").into());
    }
    Ok(())
}

pub(super) fn validate_query_definitions(
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
) -> Result<()> {
    for path in request
        .metadata_filters
        .iter()
        .map(|item| item.path.as_str())
        .chain(request.sort.iter().filter_map(|item| match &item.field {
            DocumentSortField::Metadata { path } => Some(path.as_str()),
            _ => None,
        }))
    {
        let definition = definitions
            .iter()
            .find(|item| item.field_path == path)
            .ok_or_else(|| {
                DomainError::invalid_argument(format!("metadata field '{path}' is not declared"))
            })?;
        if definition.status != "ready" {
            return Err(DomainError::invalid_argument(format!(
                "metadata field '{path}' is not ready"
            ))
            .into());
        }
        if request
            .sort
            .iter()
            .any(|item| matches!(&item.field, DocumentSortField::Metadata { path: value } if value == path))
            && !definition.sortable
        {
            return Err(
                DomainError::invalid_argument(format!("metadata field '{path}' is not sortable"))
                    .into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_query;
    use context69_contracts::{DocumentQueryRequest, DocumentSort, DocumentSortField, SortOrder};

    fn request() -> DocumentQueryRequest {
        DocumentQueryRequest {
            locale: None,
            source_key: Some("news".to_string()),
            published_after: None,
            published_before: None,
            metadata_filters: Vec::new(),
            sort: Vec::new(),
            limit: 50,
            cursor: None,
        }
    }

    #[test]
    fn limit_and_sort_failures_are_typed_invalid_argument() {
        use crate::domain_errors::{DomainError, find_domain_error};

        let assert_invalid = |error: anyhow::Error, expected: &str| {
            let message = error.to_string();
            assert!(
                matches!(
                    find_domain_error(&error),
                    Some(DomainError::InvalidArgument(_))
                ),
                "expected typed invalid_argument for {message}"
            );
            assert_eq!(message, expected);
        };

        let mut query = request();
        query.limit = 0;
        assert_invalid(
            validate_query(&query).unwrap_err(),
            "limit must be between 1 and 200",
        );
        query.limit = 201;
        assert_invalid(
            validate_query(&query).unwrap_err(),
            "limit must be between 1 and 200",
        );

        let sort = DocumentSort {
            field: DocumentSortField::PublishedAt,
            order: SortOrder::Desc,
        };
        let mut oversorted = request();
        oversorted.sort = vec![sort.clone(), sort.clone(), sort.clone(), sort];
        assert_invalid(
            validate_query(&oversorted).unwrap_err(),
            "sort supports at most 3 fields",
        );
    }
}
