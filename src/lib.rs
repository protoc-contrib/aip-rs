//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the part of AIP support that is pure data manipulation:
//! matching a resource name against its pattern, and formatting one back. It
//! also declares the traits generated code implements -- [`ResourceName`] for
//! a typed name, [`FieldBehavior`] for every message and [`Resource`] for a
//! resource -- and the [`FieldBehaviorKind`], [`FieldScope`], [`FieldPath`]
//! (each field with its number and [`FieldType`]), [`FieldError`] (a failure
//! and the field it was of) and [`FieldBehaviorError`] they work with.
//! Anything that needs to walk a protobuf message is generated instead (see
//! the scope note in the README). Behind the default `connect`
//! feature, [`FieldBehaviorInterceptor`] applies them at a Connect server's
//! boundary; without it the crate's one dependency is `buffa-types`, for the
//! `FieldMask` a resource implies. The optional `descriptor` feature adds
//! buffa-descriptor.
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
//! - AIP-134 field masks -- [`Resource::implied_field_mask`] for an update's
//!   omitted mask, [`Resource::clear_field_mask`] to remove the paths of a
//!   kind, such as `OUTPUT_ONLY`, from an explicit one
//! - AIP-203 field behavior -- [`FieldBehavior`], which clears and validates
//!   the fields of a [`FieldBehaviorKind`], within a [`FieldScope`], reporting
//!   a [`FieldBehaviorError`] with a [`FieldPath`] per field
//! - AIP-193 errors -- with the `connect` feature, the [`error`] module builds
//!   the details an error carries, the `google.rpc.Status` of a failed batch
//!   entry, and [`ErrorInterceptor`](error::ErrorInterceptor), which logs a
//!   failed call and answers it without its `DebugInfo`
//! - [`FieldBehaviorSet`] -- a set of kinds, e.g. what one field is annotated
//!   with; with the `descriptor` feature, `FieldDescriptorExt::field_behaviors`
//!   reads it off a buffa-descriptor `FieldDescriptor`
//! - AIP-133, AIP-134, AIP-233 and AIP-234 standard methods -- with the
//!   `connect` feature, [`ResourceWrites`] says which resources a request
//!   writes, and [`FieldBehaviorInterceptor`] applies the server's policy to
//!   them: clearing, filling in a create's omitted id, implying an update's
//!   omitted mask and dropping the `OUTPUT_ONLY` paths of an explicit one, and
//!   checking `REQUIRED` per method with protovalidate's rules
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

#[cfg(feature = "connect")]
pub mod error;
pub mod field;
#[cfg(feature = "connect")]
pub mod request;
pub mod resource;

#[cfg(feature = "descriptor")]
pub use field::descriptor::FieldDescriptorExt;
pub use field::{
    behavior::{
        FieldBehavior, FieldBehaviorError, FieldBehaviorKind, FieldBehaviorSet, FieldScope,
        UnknownFieldBehavior,
    },
    error::FieldError,
    path::{FieldPath, FieldResolver, FieldType, Step},
};
#[cfg(feature = "connect")]
pub use request::{
    interceptor::{FieldBehaviorInterceptor, Procedure},
    prepare::{
        PrepareError, PrepareFailure, PrepareInterceptor, Preparer, Reject, RequestPreparer,
        reject_prepare,
    },
    writes::{DynResource, ResourceWrite, ResourceWrites, WriteMethod},
};
pub use resource::{Resource, ResourceName, ResourcePattern};

// Not implemented here, deliberately:
//
//   AIP-203 field behavior is declared here as the FieldBehavior trait but
//           implemented by protoc-gen-rust-aip, because clearing fields mutates
//           a protobuf message, which buffa 0.9 offers no reflective path to --
//           and the read-only check is generated alongside, so field behavior
//           has one mechanism. Which operation a service applies, with which
//           kind, to which message, is its own policy.
//   AIP-133/134 request handling -- filling in a create's id, implying an
//           update's mask, checking REQUIRED per method across a batch -- is
//           the `connect` feature's, an opinionated policy built on these
//           traits; the traits themselves work on one message at a time.
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

#[cfg(all(test, feature = "connect"))]
mod tests {
    use connectrpc::{ConnectRpcService, Router};

    use crate::{
        FieldBehaviorInterceptor, PrepareInterceptor, Preparer, Procedure, error::ErrorInterceptor,
        reject_prepare,
    };

    static PROCEDURES: &[Procedure] = &[];
    static PREPARERS: &[Preparer] = &[];

    /// Every interceptor a service registers, built as a service builds it.
    #[test]
    fn every_interceptor_registers_on_a_service() {
        let _service = ConnectRpcService::new(Router::new())
            .with_interceptor(
                ErrorInterceptor::new()
                    .with_default_error_info("example.com")
                    .with_request_id(|_| None),
            )
            .with_interceptor(FieldBehaviorInterceptor::new(PROCEDURES))
            .with_interceptor(PrepareInterceptor::new(PREPARERS).with_reject(reject_prepare));
    }
}
