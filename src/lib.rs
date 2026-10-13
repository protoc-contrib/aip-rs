//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the part of AIP support that is pure data manipulation:
//! matching a resource name against its pattern, and formatting one back. It
//! also declares the traits generated code implements -- [`ResourceName`] for
//! a typed name, [`FieldBehavior`] for every message and [`Resource`] for a
//! resource -- and the [`FieldBehaviorKind`], [`FieldScope`], [`FieldPath`]
//! (each field with its number and [`FieldType`]) and [`FieldBehaviorError`]
//! they work with. Anything that needs to walk a protobuf message is generated
//! instead (see the scope note in the README), so the crate's one dependency
//! is `buffa-types`, for the `FieldMask` a resource implies; the optional
//! `descriptor` feature adds buffa-descriptor.
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
//! - AIP-123 resources -- [`Resource`], linking a resource to its typed name
//! - AIP-134 implied field masks -- [`Resource::implied_field_mask`]
//! - AIP-203 field behavior -- [`FieldBehavior`], which clears and validates
//!   the fields of a [`FieldBehaviorKind`], within a [`FieldScope`], reporting
//!   a [`FieldBehaviorError`] with a [`FieldPath`] per field
//! - [`FieldBehaviorSet`] -- a set of kinds, e.g. what one field is annotated
//!   with; with the `descriptor` feature, `FieldDescriptorExt::field_behaviors`
//!   reads it off a buffa-descriptor `FieldDescriptor`
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

#[cfg(feature = "descriptor")]
pub mod descriptor;
pub mod field_behavior;
pub mod field_path;
pub mod resource;

#[cfg(feature = "descriptor")]
pub use descriptor::FieldDescriptorExt;
pub use field_behavior::{
    FieldBehavior, FieldBehaviorError, FieldBehaviorKind, FieldBehaviorSet, FieldScope,
    UnknownFieldBehavior,
};
pub use field_path::{FieldPath, FieldType, Step};
pub use resource::{Resource, ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-203 field behavior is declared here as the FieldBehavior trait but
//           implemented by protoc-gen-rust-aip, because clearing fields mutates
//           a protobuf message, which buffa 0.9 offers no reflective path to --
//           and the read-only check is generated alongside, so field behavior
//           has one mechanism. Which operation a service applies, with which
//           kind, to which message, is its own policy.
//   AIP-133/134 request handling -- naming a create's resource, implying an
//           update's mask, checking REQUIRED per method across a batch -- is
//           built on these traits by the caller, not here: this crate works on
//           one message at a time.
//   AIP-134 update_mask validation is protovalidate's (`field_mask.in`), as
//           are a request's own REQUIRED fields and value rules.
//   AIP-160 filter is plain CEL; the plugin generates its environment.
//   AIP-132 order_by and AIP-158 page tokens are the query layer's: how a List
//           request sorts and where a page resumes are decided where the query
//           runs -- sqlx-query, for SQL -- and a second parser here could only
//           disagree with it.
//
// Revisit if a buffa release lands `reflect_mut`: field behavior could then
// move here behind a `buffa` feature, as aip-go does with protoreflect.
