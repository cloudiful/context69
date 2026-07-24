use anyhow::{Result, anyhow};
use context69_contracts::Pagination;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageBounds {
    pub page: u32,
    pub page_size: u32,
    pub offset: i64,
}

impl PageBounds {
    pub(crate) fn new(page: u32, page_size: u32) -> Result<Self> {
        Ok(Self {
            page,
            page_size,
            offset: Pagination::offset(page, page_size)?,
        })
    }

    pub(crate) fn pagination(self, total: i64) -> Result<Pagination> {
        let total = u64::try_from(total).map_err(|_| anyhow!("negative page count"))?;
        Pagination::try_new(self.page, self.page_size, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_page() {
        let error = PageBounds::new(0, 50).unwrap_err();
        assert_eq!(error.to_string(), "page must be greater than 0");
    }

    #[test]
    fn rejects_page_size_out_of_range() {
        let error = PageBounds::new(1, 101).unwrap_err();
        assert_eq!(error.to_string(), "page_size must be between 1 and 100");
    }

    #[test]
    fn calculates_offset_at_the_upper_bound_without_overflow() {
        let bounds = PageBounds::new(u32::MAX, 100).unwrap();
        assert_eq!(bounds.offset, 429_496_729_400);
    }

    #[test]
    fn rejects_negative_total() {
        let bounds = PageBounds::new(1, 50).unwrap();
        let error = bounds.pagination(-1).unwrap_err();
        assert_eq!(error.to_string(), "negative page count");
    }

    #[test]
    fn handles_empty_data() {
        let pagination = PageBounds::new(1, 50).unwrap().pagination(0).unwrap();
        assert_eq!(pagination.total_pages, 0);
    }

    #[test]
    fn handles_exact_pages() {
        let pagination = PageBounds::new(2, 50).unwrap().pagination(100).unwrap();
        assert_eq!(pagination.total_pages, 2);
    }

    #[test]
    fn handles_partial_pages() {
        let pagination = PageBounds::new(1, 50).unwrap().pagination(51).unwrap();
        assert_eq!(pagination.total_pages, 2);
    }
}
