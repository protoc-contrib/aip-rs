//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the part of AIP support that is pure data manipulation:
//! matching a resource name against its pattern, and formatting one back. It
//! also declares the traits generated code implements -- [`ResourceName`] and
//! [`FieldBehavior`] -- while anything that needs to walk a protobuf message is
//! generated instead (see the scope note in the README), which is why the crate
//! has no dependencies at all, not even a protobuf runtime.
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
//! - AIP-134 update masks and AIP-203 field behavior -- [`FieldBehavior`],
//!   implemented by generated code
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

pub mod field_behavior;
pub mod resource;

pub use field_behavior::FieldBehavior;
pub use resource::{ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-134 update_mask expansion and AIP-203 field behavior are declared here
//           as the FieldBehavior trait but implemented by protoc-gen-rust-aip,
//           because clearing OUTPUT_ONLY fields mutates a protobuf message,
//           which buffa 0.9 offers no reflective path to -- and the read-only
//           halves, REQUIRED and IMMUTABLE included, are generated alongside,
//           so field behavior has one mechanism.
//   AIP-134 update_mask validation is protovalidate's (`field_mask.in`), as
//           are a request's own REQUIRED fields and value rules. REQUIRED on a
//           resource is not: the resource is shared by its create and update
//           requests, and a rule on it cannot tell which one it is in.
//   AIP-160 filter is plain CEL; the plugin generates its environment.
//   AIP-132 order_by and AIP-158 page tokens are the query layer's: how a List
//           request sorts and where a page resumes are decided where the query
//           runs -- sqlx-query, for SQL -- and a second parser here could only
//           disagree with it.
//
// Revisit if a buffa release lands `reflect_mut`: all of field behavior could
// then move here behind a `buffa` feature, as aip-go does with protoreflect.
