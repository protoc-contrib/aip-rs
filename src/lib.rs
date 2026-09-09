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
//! One more contributes only the error generated code reports with, for the
//! same reason [`query`] does — the check needs a message to walk, which this
//! crate has no protobuf runtime to walk:
//!
//! - AIP-134 field masks — [`field_mask::FieldMaskError`]
//!
//! AIP-160 filtering is plain CEL, so the parser is whichever CEL crate the
//! caller picked and this one contributes only the error type generated code
//! reports with — see [`query`].
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

mod wire;

pub mod field_mask;
pub mod ordering;
pub mod pagination;
pub mod query;
pub mod resource;

pub use ordering::{OrderBy, OrderByField};
pub use pagination::{CursorValue, PageToken};
pub use query::QueryError;
pub use resource::{ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-134 update_mask validation and AIP-203 field behavior are generated,
//           because they need to walk — and in the OUTPUT_ONLY case mutate — a
//           protobuf message, which buffa 0.9.1 offers no reflective path to.
//           The mask error is here, so a caller matches on one type rather than
//           one per generated package; the walk is not.
//   AIP-160 filter is plain CEL, so the runtime is the `cel` crate and this
//           crate contributes nothing.
//   AIP-203 REQUIRED is protovalidate's, not ours. A schema that marks a field
//           REQUIRED and also constrains it with `buf.validate` has said the
//           same thing twice; enforcing it a third time from here would only
//           add a copy free to disagree with the one the server actually runs.
//           Keeping the two annotations in step is a lint, not a runtime.
//
// Doing the mask check reflectively here instead would need a buffa feature,
// and so a protobuf runtime in a crate that has gone to some trouble not to
// have one. Generating it costs output size and nothing else: the set of
// updatable paths is known at codegen time, so the walk has no descriptor to
// consult.
