//! Request building, response parsing and pagination.

use crate::types::PageInfo;

/// A paginated connection: a page of items plus the information to continue.
pub trait Page {
    type Item;
    fn page_info(&self) -> &PageInfo;
    fn into_items(self) -> Vec<Self::Item>;
}
