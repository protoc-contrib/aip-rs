//! The resources a standard-method request writes: [`ResourceWrites`], which
//! says which, and what each needs of the method that writes it.
//!
//! The Rust counterpart of aip-go's `writes.go`.

use buffa_types::FieldMask;

use crate::{FieldBehavior, FieldBehaviorKind, FieldPath, Resource};

/// A request for an AIP standard method, by the resources it writes.
///
/// protoc-gen-rust-aip implements it for every message a service takes, and
/// for the entries of a batch one. Three shapes are recognized, as aip-go
/// recognizes them by reflection:
///
/// - a create (AIP-133): a singular field whose type is a resource, and no
///   `update_mask`;
/// - an update (AIP-134): that resource field, and a `google.protobuf.FieldMask
///   update_mask` beside it -- the schema, not the value, makes it an update;
/// - a batch (AIP-233, AIP-234): a repeated `requests` of creates or updates,
///   each written under `requests[i]`.
///
/// Every other request writes nothing, which the default says.
///
/// [`validate`](super::behavior::validate) is built on it; so is a service's own policy, such as filling
/// in ids its own way.
pub trait ResourceWrites {
    /// Calls `f` with each resource this request writes: its own, or one per
    /// entry of a batch. A request whose resource is unset writes none.
    fn for_each_write(&mut self, _f: &mut dyn FnMut(ResourceWrite<'_>)) {}
}

/// One resource a standard-method request writes.
pub struct ResourceWrite<'a> {
    /// Where the resource is in the request: `item`, or `requests[1].item`.
    pub path: FieldPath,
    /// The resource.
    pub resource: &'a mut dyn DynResource,
    /// What the method needs of it.
    pub method: WriteMethod<'a>,
}

impl ResourceWrite<'_> {
    /// The write with its path prefixed with `prefix`: how a batch reports an
    /// entry's write, under `requests[i]`.
    #[must_use]
    pub fn prefixed(self, prefix: &FieldPath) -> Self {
        Self {
            path: prefix.clone().join(self.path),
            ..self
        }
    }
}

/// Which standard method a [`ResourceWrite`] is for, with the request fields
/// it needs.
pub enum WriteMethod<'a> {
    /// A create (AIP-133).
    Create {
        /// The request's `<resource>_id`, empty when the client left the id to
        /// the server; `None` when the request has no such field.
        id: Option<&'a mut String>,
    },
    /// An update (AIP-134).
    Update {
        /// The request's `update_mask`, with no paths when the client omitted
        /// it. Reaching it makes the mask present, which an implied mask then
        /// fills.
        mask: &'a mut FieldMask,
    },
}

/// What a [`ResourceWrite`] needs of its resource, in a form a `&mut dyn` can
/// hold: every [`Resource`] is one.
pub trait DynResource: FieldBehavior {
    /// [`Resource::implied_field_mask`].
    fn implied_field_mask(&self) -> FieldMask;

    /// [`Resource::clear_field_mask`].
    fn clear_field_mask(&self, kind: FieldBehaviorKind, mask: &mut FieldMask);
}

impl<T: Resource> DynResource for T {
    fn implied_field_mask(&self) -> FieldMask {
        Resource::implied_field_mask(self)
    }

    fn clear_field_mask(&self, kind: FieldBehaviorKind, mask: &mut FieldMask) {
        Resource::clear_field_mask(self, kind, mask);
    }
}
