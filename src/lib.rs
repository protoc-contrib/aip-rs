//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the part of AIP support that is pure data manipulation:
//! matching a resource name against its pattern, and formatting one back.
//! Anything that needs to walk a protobuf message is generated instead -- see
//! the scope note in the README -- which is why the crate has no dependencies
//! at all, not even a protobuf runtime.
//!
//! ```
//! use aip::ResourcePattern;
//!
//! let pattern: ResourcePattern = "publishers/{publisher}/books/{book}".parse()?;
//!
//! let ids = pattern.scan("publishers/p1/books/b1")?;
//! assert_eq!(ids, ["p1", "b1"]);
//! assert_eq!(pattern.format(&ids), "publishers/p1/books/b1");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! # The AIPs implemented here
//!
//! - AIP-122 resource names -- [`ResourcePattern`], [`ResourceName`]
//! - AIP-159 wildcards -- [`resource::WILDCARD`]
//!
//! # Names
//!
//! The crate is published as `aip-rs` because `aip` is taken on crates.io, but
//! its library name is `aip`, so generated code reads `aip::ResourcePattern`.
//! The type you reach for is re-exported at the crate root; the error types
//! stay in [`resource`], where they are rarely named and easy to find.
//!
//! See <https://google.aip.dev>.

#![deny(missing_docs)]

pub mod resource;

pub use resource::{ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-134 update_mask validation and AIP-203 field behavior are generated,
//           because they need to walk — and in the OUTPUT_ONLY case mutate — a
//           protobuf message, which buffa 0.9.1 offers no reflective path to.
//   AIP-132 order_by, AIP-158 page tokens and AIP-160 filter are the query
//           layer's: what a List request may name, how it sorts and where a
//           page resumes are decided where the query runs -- sqlx-query, for
//           SQL -- and a second parser here could only disagree with it.
//
// The read-only halves of AIP-134 and AIP-203 could live here behind a buffa
// feature; see the README.
