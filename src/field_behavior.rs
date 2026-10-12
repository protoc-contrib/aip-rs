//! AIP-133 resource names, AIP-134 update masks and AIP-203 field behavior, as
//! one trait.
//!
//! See [`FieldBehavior`], and the errors its checks return: [`RequiredError`],
//! [`ImmutableError`].

use core::fmt;

use crate::FieldPath;

/// What `google.api.field_behavior` says a message's fields may do, and what
/// AIP leaves to the server on a request: which fields an update may write,
/// which a create or update must not leave unset, which may never change,
/// which one side of the wire may not see, and what a create's resource is
/// named and an omitted update mask covers.
///
/// Implemented by `protoc-gen-rust-aip` for every message it generates — a
/// resource for the update-mask and check items, a create, update or batch
/// request for the request-level items, a request or a message nested in one
/// for the clearing walks, and an empty impl for a message with none of these.
/// An implementation contains only the items its message needs. Every default
/// is the "nothing to do" answer — no writable paths, nothing to imply, nothing
/// unset, nothing changed, nothing to clear — so generic code works for every
/// generated message: one server layer over `T: FieldBehavior` can clear,
/// imply and validate every request it handles.
///
/// The items are generated rather than reflective because buffa 0.9 has no
/// reflective path to mutation, and the read-only ones are generated alongside
/// so field behavior has one mechanism.
///
/// The items are named after what they do: `validate_*` returns `Ok(())` or the
/// fields that fail, `imply_*` fills in what a client may leave to the server,
/// `clear_*` drops what one side of the wire may not see.
///
/// Not dyn-compatible: [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) is an associated
/// constant and [`validate_immutable`](Self::validate_immutable) takes `&Self`.
/// Use generics, and bring the trait into scope where its methods are called:
///
/// ```
/// use aip::{FieldBehavior, RequiredError};
///
/// #[derive(Default)]
/// struct Book {
///     title: String,
///     create_time: Option<u64>,
/// }
///
/// impl FieldBehavior for Book {
///     const MUTABLE_PATHS: &'static [&'static str] = &["title"];
///
///     fn validate_required_fields(&self, mask: Option<&[String]>) -> Result<(), RequiredError> {
///         let covered = mask.is_none_or(|paths| paths.iter().any(|path| path == "title"));
///         RequiredError::check(if covered && self.title.is_empty() { vec!["title".into()] } else { Vec::new() })
///     }
///
///     fn clear_output_only(&mut self) {
///         self.create_time = None;
///     }
/// }
///
/// /// One layer over every resource type.
/// fn admit<T: FieldBehavior>(mut resource: T) -> Result<T, RequiredError> {
///     resource.clear_output_only();
///     resource.validate_required_fields(None)?;
///     Ok(resource)
/// }
///
/// let error = admit(Book::default()).err().unwrap();
/// assert_eq!(error.to_string(), "required fields are unset: title");
/// assert!(Book::default().implied_update_mask().is_empty()); // the default
/// ```
///
/// See <https://google.aip.dev/133>, <https://google.aip.dev/134> and
/// <https://google.aip.dev/203>.
pub trait FieldBehavior {
    /// The field paths an update may write: every field not annotated
    /// `OUTPUT_ONLY`, `IDENTIFIER` or `IMMUTABLE`, in declaration order.
    ///
    /// This is the *expansion* of the `*` mask, full replacement, which AIP-134
    /// reads as every writable field; an omitted mask is narrower — see
    /// [`implied_update_mask`](Self::implied_update_mask). It is not the check
    /// that a mask names only these: that is `(buf.validate.field).field_mask.in`
    /// on the mask itself, which protovalidate runs along with every other rule
    /// on the request.
    ///
    /// Those two lists say the same thing and are kept in step by a lint
    /// (protoc-gen-aip-lint's `update-mask-writable-fields`).
    ///
    /// Empty by default: a message with no writable fields, or that is not a
    /// resource.
    ///
    /// See <https://google.aip.dev/134#full-replacement>.
    const MUTABLE_PATHS: &'static [&'static str] = &[];

    /// The update mask AIP-134 implies when a client omits one: every field of
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) this message populates, in
    /// declaration order.
    ///
    /// Populated as protobuf reads presence: a field with explicit presence —
    /// `optional`, a message, a oneof member — when set, even to its zero value;
    /// any other scalar when not its zero value; a repeated field or a map when
    /// not empty. So a zero value without explicit presence is never implied: a
    /// client clears one with an explicit mask, or the schema makes the field
    /// `optional`.
    ///
    /// A populated message field is implied as one path — `carrier`, not
    /// `carrier.name` — so the update writes everything beneath it, nested
    /// `OUTPUT_ONLY` values included. Clear, then imply: call
    /// [`clear_output_only`](Self::clear_output_only) first.
    ///
    /// Reads `self` and returns the paths;
    /// [`imply_update_mask`](Self::imply_update_mask) stores them on a request.
    /// Empty by default.
    ///
    /// See <https://google.aip.dev/134#field-masks>.
    #[must_use]
    fn implied_update_mask(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Names the resource a create request carries, and each create entry of a
    /// batch request (AIP-233), as AIP-133 leaves to the server:
    ///
    /// - an omitted `<resource>_id` becomes `new_id()`; an id the client sent
    ///   is kept as is, its format being for the request's protovalidate rules;
    /// - the resource's `parent` field, if it has one, is copied from the
    ///   request's `parent`;
    /// - the resource's `name` is built from its `google.api.resource` pattern,
    ///   the request's `parent` and that id, replacing any name the client sent:
    ///   `items/{item}` gives `items/<id>`, and
    ///   `collections/{collection}/items/{item}` gives `<parent>/items/<id>`.
    ///
    /// A create request without a `<resource>_id` field is left as is — its
    /// handler names the resource — and so is an update, whose name selects its
    /// target, and any other request. `new_id` mints the id: the caller's
    /// choice of scheme, such as a UUIDv7 so ids sort by creation time.
    ///
    /// aip-go's `ImplyResourceName` does the same. Does nothing by default.
    ///
    /// See <https://google.aip.dev/133#user-specified-ids>.
    fn imply_resource_name(&mut self, _new_id: &mut dyn FnMut() -> String) {}

    /// Sets, on an update request (AIP-134) that omitted its `update_mask` —
    /// unset, or with no paths — and whose resource is set, the mask the
    /// resource's [`implied_update_mask`](Self::implied_update_mask) implies;
    /// on a batch request (AIP-235), on each such entry of `requests`. A mask
    /// the client sent is left as is, and a create or any other request is
    /// untouched.
    ///
    /// Clear the request's `OUTPUT_ONLY` fields first, with
    /// [`clear_output_only`](Self::clear_output_only), so the implied mask never
    /// names a value a client tried to set on one.
    ///
    /// aip-go's `ImplyUpdateMask` does the same. Does nothing by default.
    ///
    /// See <https://google.aip.dev/134#field-masks>.
    fn imply_update_mask(&mut self) {}

    /// Fills in what AIP lets a client leave out of a create, update or batch
    /// request: a create's resource name
    /// ([`imply_resource_name`](Self::imply_resource_name)), then an update's
    /// omitted mask ([`imply_update_mask`](Self::imply_update_mask)).
    ///
    /// ```text
    /// request.clear_output_only();
    /// request.imply_request(&mut || Uuid::now_v7().to_string());
    /// request.validate_required()?;
    /// ```
    ///
    /// aip-go's `ImplyRequest` does the same. Implemented here, from the two
    /// items it calls.
    fn imply_request(&mut self, new_id: &mut dyn FnMut() -> String) {
        self.imply_resource_name(new_id);
        self.imply_update_mask();
    }

    /// Checks that this request, an AIP standard method request, sets every
    /// `REQUIRED` field the method asks for, and returns the unset ones as
    /// paths from the request, in the order it holds them:
    ///
    /// - a create (AIP-133), whose resource is its field of a resource type:
    ///   every `REQUIRED` field of the resource, as
    ///   [`validate_required_fields`](Self::validate_required_fields) with
    ///   `None` reads them;
    /// - an update (AIP-134), a request whose type has an `update_mask`: only
    ///   those the mask names, so a partial update can't clear one. The schema,
    ///   not the value, says which method this is: an unset mask names nothing,
    ///   and is never read as a create;
    /// - a batch (AIP-233, AIP-235): each entry of `requests` as its own
    ///   request, under `requests[i]`.
    ///
    /// Any other request is valid, and so is an unset resource, which the
    /// request's own `(buf.validate.field).required` reports. Call
    /// [`imply_request`](Self::imply_request) first, or an update that omitted
    /// its mask names nothing:
    ///
    /// ```text
    /// request.imply_request(&mut new_id);
    /// if let Err(error) = request.validate_required() {
    ///     // error.paths: ["requests[1].item.display_name"]
    /// }
    /// ```
    ///
    /// aip-go's `ValidateRequired` answers the same. `Ok` by default: not a
    /// create, update or batch request.
    ///
    /// See <https://google.aip.dev/203#required>.
    fn validate_required(&self) -> Result<(), RequiredError> {
        Ok(())
    }

    /// Checks that this message, a resource, sets its `REQUIRED` fields, and
    /// returns the unset ones in declaration order.
    ///
    /// AIP-203: a create must set every `REQUIRED` field, so `None` checks them
    /// all. A partial update may omit any of them, but one its mask names must
    /// be set, or the update would clear it, so `Some(paths)` checks only the
    /// fields the mask names — a path naming a message checks everything
    /// beneath it, a nested path just that field. Expand `*` with
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) first. The parameter is the owned
    /// `FieldMask`'s `paths` as it is, so a request's mask is passed without
    /// conversion:
    ///
    /// ```text
    /// item.validate_required_fields(None)?; // create
    /// item.validate_required_fields(Some(&update_mask.paths))?; // update
    /// ```
    ///
    /// Unset is protobuf presence, as
    /// [`implied_update_mask`](Self::implied_update_mask) reads it: a
    /// `REQUIRED` `bool` without explicit presence must be `true`.
    ///
    /// A set singular message is descended into, its own `REQUIRED` fields
    /// reported as dotted paths (`address.city`) — a nested message's
    /// annotations are independent of its parent's. An unset one is not entered
    /// on a create, or when the mask names it whole; its own `REQUIRED` is
    /// reported at its level. But a nested path is checked even when its parent
    /// is unset, reading the parent as empty: the update writes that path, so
    /// it would clear it. Repeated and map fields are never descended into. A
    /// message type that contains itself is descended into once per path.
    ///
    /// This is where `REQUIRED` on a resource is enforced, rather than by
    /// protovalidate: a resource message is shared by its create and update
    /// requests, and a rule on it cannot tell which one it is in. protovalidate
    /// keeps a request's own fields and value rules on the resource's. A server
    /// reads it per request, with [`validate_required`](Self::validate_required).
    ///
    /// aip-go's `ValidateRequiredFields` answers the same. `Ok` by default:
    /// nothing is `REQUIRED`.
    ///
    /// See <https://google.aip.dev/203#required>.
    fn validate_required_fields(&self, _mask: Option<&[String]>) -> Result<(), RequiredError> {
        Ok(())
    }

    /// Checks that this update leaves every `IMMUTABLE` field as `existing`
    /// holds it, and returns the ones it changes, in declaration order.
    ///
    /// AIP-203: a service rejects a request that changes an immutable field. An
    /// update that echoes the stored value back, or leaves the field unset,
    /// changes nothing and is valid. Neither the implied mask nor
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) ever writes one, so without this
    /// check a changed value would be dropped silently rather than refused. A
    /// repeated field that is also `UNORDERED_LIST` is compared ignoring order,
    /// as a multiset.
    ///
    /// ```text
    /// update.validate_immutable(&stored)?; // InvalidArgument on Err
    /// ```
    ///
    /// aip-go's `ValidateImmutable` answers the same. `Ok` by default: nothing
    /// is `IMMUTABLE`.
    ///
    /// See <https://google.aip.dev/203#immutable>.
    fn validate_immutable(&self, _existing: &Self) -> Result<(), ImmutableError> {
        Ok(())
    }

    /// Clears every field annotated `OUTPUT_ONLY`, at any depth.
    ///
    /// Call it on an inbound request to drop server-owned values a client
    /// should not be able to set, rather than rejecting the request. A `oneof`
    /// is cleared entirely when the member that is set is output-only.
    ///
    /// Does nothing by default.
    ///
    /// See <https://google.aip.dev/203#output-only>.
    fn clear_output_only(&mut self) {}

    /// Clears every field annotated `INPUT_ONLY`, at any depth.
    ///
    /// Call it on an outbound response, so a value the client may send is never
    /// returned. A `oneof` is cleared entirely when the member that is set is
    /// input-only.
    ///
    /// Does nothing by default.
    ///
    /// See <https://google.aip.dev/203#input-only>.
    fn clear_input_only(&mut self) {}
}

/// The `REQUIRED` fields a message or request leaves unset (AIP-203), as
/// [`FieldBehavior::validate_required`] and
/// [`FieldBehavior::validate_required_fields`] report them.
///
/// ```
/// use aip::{FieldPath, RequiredError};
///
/// let error = RequiredError::new(vec!["display_name".into()]).under(FieldPath::field("item"));
/// assert_eq!(error.to_string(), "required fields are unset: item.display_name");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredError {
    /// The unset fields, from the message checked, in the order it holds them.
    /// Never empty.
    pub paths: Vec<FieldPath>,
}

impl RequiredError {
    /// The error for `paths`, which must not be empty: use
    /// [`check`](Self::check) when it may be.
    #[must_use]
    pub fn new(paths: Vec<FieldPath>) -> Self {
        debug_assert!(
            !paths.is_empty(),
            "a RequiredError names at least one field"
        );
        Self { paths }
    }

    /// `Ok(())` when `paths` is empty, and the error for them otherwise.
    ///
    /// # Errors
    ///
    /// Returns a `RequiredError` holding `paths` when it is not empty.
    pub fn check(paths: Vec<FieldPath>) -> Result<(), Self> {
        if paths.is_empty() {
            Ok(())
        } else {
            Err(Self { paths })
        }
    }

    /// This error with every path continued from `prefix`: how a request
    /// reports its resource's fields (`item.display_name`), and a batch its
    /// entries' (`requests[1].item.display_name`).
    #[must_use]
    pub fn under(self, prefix: FieldPath) -> Self {
        Self {
            paths: self
                .paths
                .into_iter()
                .map(|path| prefix.clone().join(path))
                .collect(),
        }
    }
}

/// Writes `required fields are unset: ` and the dotted paths, comma-separated.
impl fmt::Display for RequiredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("required fields are unset: ")?;
        write_paths(f, &self.paths)
    }
}

impl std::error::Error for RequiredError {}

/// The `IMMUTABLE` fields an update changes (AIP-203), as
/// [`FieldBehavior::validate_immutable`] reports them.
///
/// ```
/// use aip::ImmutableError;
///
/// let error = ImmutableError::new(vec!["region".into()]);
/// assert_eq!(error.to_string(), "immutable fields are changed: region");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableError {
    /// The changed fields, in declaration order. Never empty.
    pub paths: Vec<FieldPath>,
}

impl ImmutableError {
    /// The error for `paths`, which must not be empty: use
    /// [`check`](Self::check) when it may be.
    #[must_use]
    pub fn new(paths: Vec<FieldPath>) -> Self {
        debug_assert!(
            !paths.is_empty(),
            "an ImmutableError names at least one field"
        );
        Self { paths }
    }

    /// `Ok(())` when `paths` is empty, and the error for them otherwise.
    ///
    /// # Errors
    ///
    /// Returns an `ImmutableError` holding `paths` when it is not empty.
    pub fn check(paths: Vec<FieldPath>) -> Result<(), Self> {
        if paths.is_empty() {
            Ok(())
        } else {
            Err(Self { paths })
        }
    }
}

/// Writes `immutable fields are changed: ` and the dotted paths,
/// comma-separated.
impl fmt::Display for ImmutableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("immutable fields are changed: ")?;
        write_paths(f, &self.paths)
    }
}

impl std::error::Error for ImmutableError {}

fn write_paths(f: &mut fmt::Formatter<'_>, paths: &[FieldPath]) -> fmt::Result {
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{path}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FieldBehavior, ImmutableError, RequiredError};
    use crate::FieldPath;

    /// A message with no field behavior at all: every item is the default.
    #[derive(Default)]
    struct Plain;

    impl FieldBehavior for Plain {}

    #[test]
    fn every_default_is_nothing_to_do() {
        let mut plain = Plain;
        assert!(Plain::MUTABLE_PATHS.is_empty());
        assert!(plain.implied_update_mask().is_empty());
        assert_eq!(plain.validate_required_fields(None), Ok(()));
        assert_eq!(
            plain.validate_required_fields(Some(&["x".to_owned()])),
            Ok(())
        );
        assert_eq!(plain.validate_immutable(&Plain), Ok(()));
        assert_eq!(plain.validate_required(), Ok(()));
        let mut minted = 0;
        plain.imply_request(&mut || {
            minted += 1;
            "id".to_owned()
        });
        assert_eq!(minted, 0, "the default names nothing, so mints nothing");
        plain.imply_resource_name(&mut || unreachable!());
        plain.imply_update_mask();
        plain.clear_output_only();
        plain.clear_input_only();
    }

    /// Overrides one item and keeps the rest.
    #[derive(Default)]
    struct Stamped {
        create_time: Option<u64>,
    }

    impl FieldBehavior for Stamped {
        fn clear_output_only(&mut self) {
            self.create_time = None;
        }
    }

    fn admit<T: FieldBehavior>(message: &mut T) -> Result<(), RequiredError> {
        message.clear_output_only();
        message.validate_required_fields(None)
    }

    #[test]
    fn generic_code_runs_overrides_and_defaults_alike() {
        let mut stamped = Stamped {
            create_time: Some(1),
        };
        assert_eq!(admit(&mut stamped), Ok(()));
        assert_eq!(stamped.create_time, None);
        assert_eq!(admit(&mut Plain), Ok(()));
    }

    #[test]
    fn errors_list_their_paths() {
        let required = RequiredError::new(vec!["display_name".into(), "quantity".into()])
            .under(FieldPath::field("item"));
        assert_eq!(
            required.to_string(),
            "required fields are unset: item.display_name, item.quantity"
        );
        let immutable = ImmutableError::new(vec!["region".into(), "tags".into()]);
        assert_eq!(
            immutable.to_string(),
            "immutable fields are changed: region, tags"
        );
    }

    #[test]
    fn check_is_ok_for_no_paths() {
        assert_eq!(RequiredError::check(Vec::new()), Ok(()));
        assert_eq!(ImmutableError::check(Vec::new()), Ok(()));
        assert_eq!(
            RequiredError::check(vec!["title".into()]),
            Err(RequiredError::new(vec!["title".into()]))
        );
    }

    #[test]
    fn under_nests_a_batch_entry() {
        let error = RequiredError::new(vec!["title".into()])
            .under(FieldPath::field("book"))
            .under(FieldPath::index("requests", 1));
        assert_eq!(error.paths[0].to_string(), "requests[1].book.title");
    }

    #[test]
    fn errors_are_std_errors() {
        fn boxed(error: impl std::error::Error + 'static) -> Box<dyn std::error::Error> {
            Box::new(error)
        }
        assert!(
            boxed(RequiredError::new(vec!["a".into()]))
                .source()
                .is_none()
        );
        assert!(
            boxed(ImmutableError::new(vec!["a".into()]))
                .source()
                .is_none()
        );
    }
}

#[cfg(test)]
mod request_tests {
    use super::{FieldBehavior, RequiredError};
    use crate::FieldPath;

    /// A resource with one `REQUIRED` field, `title`, named `books/{book}`.
    #[derive(Default, Clone, Debug, PartialEq)]
    struct Book {
        name: String,
        title: String,
    }

    impl FieldBehavior for Book {
        fn implied_update_mask(&self) -> Vec<&'static str> {
            if self.title.is_empty() {
                Vec::new()
            } else {
                vec!["title"]
            }
        }

        fn validate_required_fields(&self, mask: Option<&[String]>) -> Result<(), RequiredError> {
            let covered = mask.is_none_or(|paths| paths.iter().any(|path| path == "title"));
            RequiredError::check(if covered && self.title.is_empty() {
                vec!["title".into()]
            } else {
                Vec::new()
            })
        }
    }

    /// A create request, written as the plugin generates one.
    #[derive(Default)]
    struct CreateBookRequest {
        book_id: String,
        book: Option<Book>,
    }

    impl FieldBehavior for CreateBookRequest {
        fn imply_resource_name(&mut self, new_id: &mut dyn FnMut() -> String) {
            if self.book_id.is_empty() {
                self.book_id = new_id();
            }
            if let Some(book) = &mut self.book {
                book.name = format!("books/{}", self.book_id);
            }
        }

        fn validate_required(&self) -> Result<(), RequiredError> {
            self.book.as_ref().map_or(Ok(()), |book| {
                book.validate_required_fields(None)
                    .map_err(|error| error.under(FieldPath::field("book")))
            })
        }
    }

    /// An update request, written as the plugin generates one.
    #[derive(Default)]
    struct UpdateBookRequest {
        book: Option<Book>,
        update_mask: Option<Vec<String>>,
    }

    impl FieldBehavior for UpdateBookRequest {
        fn validate_required(&self) -> Result<(), RequiredError> {
            let mask = self.update_mask.as_deref().unwrap_or(&[]);
            self.book.as_ref().map_or(Ok(()), |book| {
                book.validate_required_fields(Some(mask))
                    .map_err(|error| error.under(FieldPath::field("book")))
            })
        }

        fn imply_update_mask(&mut self) {
            let omitted = self.update_mask.as_ref().is_none_or(Vec::is_empty);
            if let (true, Some(book)) = (omitted, &self.book) {
                self.update_mask = Some(
                    book.implied_update_mask()
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                );
            }
        }
    }

    /// A batch of updates, written as the plugin generates one.
    #[derive(Default)]
    struct BatchUpdateBooksRequest {
        requests: Vec<UpdateBookRequest>,
    }

    impl FieldBehavior for BatchUpdateBooksRequest {
        fn validate_required(&self) -> Result<(), RequiredError> {
            let paths = self
                .requests
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    entry
                        .validate_required()
                        .err()
                        .map(|error| error.under(FieldPath::index("requests", i)))
                })
                .flat_map(|error| error.paths)
                .collect();
            RequiredError::check(paths)
        }

        fn imply_update_mask(&mut self) {
            for entry in &mut self.requests {
                entry.imply_update_mask();
            }
        }
    }

    #[test]
    fn a_batch_reports_each_entry_under_its_index() {
        let mut request = BatchUpdateBooksRequest {
            requests: vec![
                UpdateBookRequest {
                    book: Some(Book {
                        title: "Dune".into(),
                        ..Book::default()
                    }),
                    update_mask: None,
                },
                UpdateBookRequest {
                    book: Some(Book::default()),
                    update_mask: Some(vec!["title".into()]),
                },
            ],
        };
        request.imply_request(&mut || unreachable!("an update names nothing"));
        assert_eq!(
            request.requests[0].update_mask,
            Some(vec!["title".to_owned()])
        );
        let error = request.validate_required().unwrap_err();
        assert_eq!(
            error.to_string(),
            "required fields are unset: requests[1].book.title"
        );
    }

    #[test]
    fn imply_request_names_a_create_from_a_minted_id() {
        let mut request = CreateBookRequest {
            book: Some(Book {
                title: "Dune".into(),
                ..Book::default()
            }),
            ..CreateBookRequest::default()
        };
        request.imply_request(&mut || "b1".to_owned());
        assert_eq!(request.book_id, "b1");
        assert_eq!(request.book.as_ref().unwrap().name, "books/b1");
        assert_eq!(request.validate_required(), Ok(()));
    }

    #[test]
    fn a_create_reports_its_resource_under_its_field() {
        let request = CreateBookRequest {
            book: Some(Book::default()),
            ..CreateBookRequest::default()
        };
        assert_eq!(
            request.validate_required().unwrap_err().to_string(),
            "required fields are unset: book.title"
        );
    }
}
