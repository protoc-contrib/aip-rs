//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the parts of AIP support that are pure data manipulation:
//! encoding a page token, parsing an `order_by`, matching a resource name
//! against its pattern. Anything that needs to walk a protobuf message is
//! generated instead — see the scope note in the README — which is why the
//! crate has no dependencies at all, not even a protobuf runtime.
//!
//! The pieces compose into one List handler:
//!
//! ```
//! use aip::{OrderBy, PageToken};
//!
//! # struct Book { title: String }
//! # fn query(_: &PageToken, _: &OrderBy, _: i32) -> Vec<Book> { Vec::new() }
//! # fn handler(order_by: &str, page_token: &str, page_size: i32, checksum: u32)
//! #     -> Result<(Vec<Book>, String), Box<dyn std::error::Error>> {
//! let order_by: OrderBy = order_by.parse()?;
//! order_by.validate_for_paths(["title", "create_time"])?;
//! let token = PageToken::parse(page_token, checksum)?;
//!
//! let books = query(&token, &order_by, page_size);
//!
//! let mut next_page_token = String::new();
//! if books.len() == page_size as usize {
//!     let last = books.last().expect("a full page is not empty");
//!     next_page_token = token.next_cursor(vec![last.title.as_str().into()])?.encode();
//! }
//! # Ok((books, next_page_token))
//! # }
//! ```
//!
//! # The AIPs implemented here
//!
//! - AIP-122 resource names — [`ResourcePattern`], [`ResourceName`]
//! - AIP-132 ordering — [`OrderBy`]
//! - AIP-158 pagination — [`PageToken`], [`CursorValue`]
//!
//! # Names
//!
//! The crate is published as `aip-rs` because `aip` is taken on crates.io, but
//! its library name is `aip`, so generated code reads `aip::PageToken`. Each
//! AIP gets a module, and the type you reach for is re-exported at the crate
//! root; the error types stay in their modules, where they are rarely named
//! and easy to find.
//!
//! See <https://google.aip.dev>.

#![deny(missing_docs)]

mod base64url;
mod crc32;
mod varint;

pub mod ordering;
pub mod paging;
pub mod resource;

pub use ordering::{OrderBy, OrderByField};
pub use paging::{CursorValue, PageToken};
pub use resource::{ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-134 update_mask validation and AIP-203 field behavior are generated,
//           because they need to walk — and in the OUTPUT_ONLY case mutate — a
//           protobuf message, which buffa 0.9.1 offers no reflective path to.
//   AIP-160 filter is plain CEL, so the runtime is the `cel` crate and this
//           crate contributes nothing.
//
// The read-only halves of AIP-134 and AIP-203 could live here behind a buffa
// feature; see the README.
