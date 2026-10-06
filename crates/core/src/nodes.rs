//! Macros for the `{ nodes }` / `{ nodes, pageInfo }` connection wrappers.
//!
//! GraphQL connections need one Rust struct per node type, so they are
//! generated. Wrappers deref to a slice and convert into a `Vec`, so callers
//! never have to name `.nodes`.

/// A connection that only selects `nodes` (nested, bounded by `first`).
macro_rules! nodes_container {
    ($(#[$meta:meta])* $name:ident, $graphql:literal, $item:ty) => {
        $(#[$meta])*
        #[derive(cynic::QueryFragment, Debug, Clone, PartialEq, serde::Serialize)]
        #[cynic(graphql_type = $graphql)]
        pub struct $name {
            pub nodes: Vec<$item>,
        }

        impl std::ops::Deref for $name {
            type Target = [$item];
            fn deref(&self) -> &[$item] {
                &self.nodes
            }
        }

        impl From<$name> for Vec<$item> {
            fn from(value: $name) -> Self {
                value.nodes
            }
        }

        impl IntoIterator for $name {
            type Item = $item;
            type IntoIter = std::vec::IntoIter<$item>;
            fn into_iter(self) -> Self::IntoIter {
                self.nodes.into_iter()
            }
        }
    };
}

/// A connection that selects `nodes` and `pageInfo` (a paginated list).
macro_rules! paged_container {
    ($(#[$meta:meta])* $name:ident, $graphql:literal, $item:ty) => {
        $(#[$meta])*
        #[derive(cynic::QueryFragment, Debug, Clone, PartialEq, serde::Serialize)]
        #[cynic(graphql_type = $graphql)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            pub nodes: Vec<$item>,
            pub page_info: $crate::types::PageInfo,
        }

        impl $crate::wire::Page for $name {
            type Item = $item;
            fn page_info(&self) -> &$crate::types::PageInfo {
                &self.page_info
            }
            fn into_items(self) -> Vec<$item> {
                self.nodes
            }
        }
    };
}

pub(crate) use nodes_container;
pub(crate) use paged_container;
