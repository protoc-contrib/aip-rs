//! AIP-122 resource names: compiling a name pattern once, then scanning and
//! formatting against it.
//!
//! See <https://google.aip.dev/122>.

use core::fmt;
use core::str::FromStr;

use buffa_types::FieldMask;

use crate::{FieldBehavior, FieldBehaviorKind, FieldScope};

/// The AIP-159 wildcard, accepted in place of a resource ID to mean "across
/// all values of this segment".
///
/// See <https://google.aip.dev/159>.
pub const WILDCARD: &str = "-";

/// A compiled resource name pattern, such as
/// `publishers/{publisher}/books/{book}`.
///
/// This is the machinery behind the resource name types emitted by
/// `protoc-gen-rust-aip`, not a replacement for them: generated code compiles
/// its pattern once and delegates the segment walking here, so callers keep
/// working with concrete `BookName` values rather than pattern strings.
///
/// Compiling allocates, so a generated pattern belongs in a [`LazyLock`]
/// rather than a `const`. A pattern that fails to compile is a codegen bug,
/// which is what makes `expect` the right call here and nowhere else:
///
/// ```
/// use std::sync::LazyLock;
/// use aip::ResourcePattern;
///
/// static BOOK_NAME: LazyLock<ResourcePattern> = LazyLock::new(|| {
///     "publishers/{publisher}/books/{book}"
///         .parse()
///         .expect("book name pattern")
/// });
///
/// let ids = BOOK_NAME.scan("publishers/p1/books/b1")?;
/// assert_eq!(ids, ["p1", "b1"]);
/// assert_eq!(BOOK_NAME.format(&ids), "publishers/p1/books/b1");
/// # Ok::<_, aip::resource::ParseError>(())
/// ```
///
/// A compiled pattern is immutable and safe to share across threads.
///
/// [`LazyLock`]: std::sync::LazyLock
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePattern {
    pattern: String,
    segments: Vec<Segment>,
    /// How many of `segments` are variables, cached because every scan and
    /// format checks its arity against it.
    variables: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// Text the name must match exactly.
    Literal(String),
    /// A segment that captures a value, by its variable name (without
    /// braces).
    Variable(String),
}

impl Segment {
    /// The variable name, if this segment captures a value.
    fn variable(&self) -> Option<&str> {
        match self {
            Self::Literal(_) => None,
            Self::Variable(name) => Some(name),
        }
    }
}

impl ResourcePattern {
    /// Compiles a resource name pattern.
    ///
    /// A segment written as `{name}` captures a value; every other segment is
    /// a literal that must match exactly. Variable names must be unique so
    /// that error messages can identify a segment unambiguously.
    ///
    /// Segments are held to AIP-122's spelling: a literal is a `camelCase`
    /// collection identifier and a variable name is `snake_case`, each
    /// starting with a lowercase ASCII letter. Anything else — `{book=**}`
    /// path-template syntax included — is rejected here rather than turning
    /// up later as a segment that nothing can name.
    ///
    /// [`str::parse`] and [`TryFrom<&str>`](TryFrom) are the same thing, and
    /// usually read better.
    pub fn compile(pattern: &str) -> Result<Self, CompileError> {
        let error = |kind| CompileError {
            pattern: pattern.to_owned(),
            kind,
        };
        if pattern.is_empty() {
            return Err(error(CompileErrorKind::EmptyPattern));
        }
        let mut segments: Vec<Segment> = Vec::new();
        for (index, part) in pattern.split('/').enumerate() {
            let malformed = || error(CompileErrorKind::MalformedSegment(part.to_owned()));
            let Some(name) = part.strip_prefix('{') else {
                if part.is_empty() {
                    return Err(error(CompileErrorKind::EmptySegment(index)));
                }
                if has_brace(part) {
                    return Err(malformed());
                }
                if !is_collection_id(part) {
                    return Err(error(CompileErrorKind::InvalidLiteral(part.to_owned())));
                }
                segments.push(Segment::Literal(part.to_owned()));
                continue;
            };
            let Some(name) = name.strip_suffix('}') else {
                return Err(malformed());
            };
            if name.is_empty() {
                return Err(error(CompileErrorKind::EmptyVariableName(index)));
            }
            if has_brace(name) {
                return Err(malformed());
            }
            if !is_variable_name(name) {
                return Err(error(CompileErrorKind::InvalidVariableName(
                    name.to_owned(),
                )));
            }
            // Linear, but a pattern has a handful of segments.
            if segments
                .iter()
                .any(|segment| segment.variable() == Some(name))
            {
                return Err(error(CompileErrorKind::DuplicateVariable(name.to_owned())));
            }
            segments.push(Segment::Variable(name.to_owned()));
        }
        let variables = segments.iter().filter_map(Segment::variable).count();
        Ok(Self {
            pattern: pattern.to_owned(),
            segments,
            variables,
        })
    }

    /// Returns the pattern this was compiled from.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Returns the number of variable segments in the pattern.
    #[must_use]
    pub fn variables(&self) -> usize {
        self.variables
    }

    /// Returns the variable segment names, in pattern order.
    pub fn variable_names(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(Segment::variable)
    }

    /// Matches `name` against the pattern and returns its variable segments,
    /// in pattern order.
    ///
    /// The values borrow from `name`. Literal segments must match exactly and
    /// variable segments must be non-empty.
    pub fn scan<'a>(&self, name: &'a str) -> Result<Vec<&'a str>, ParseError> {
        let mut values = vec![""; self.variables];
        self.scan_into(name, &mut values)?;
        Ok(values)
    }

    /// Matches `name` against the pattern and writes its variable segments
    /// into `values`, in pattern order.
    ///
    /// The allocation-free form of [`scan`](Self::scan), for generated code
    /// that knows the arity and can hand over a fixed-size array.
    ///
    /// On failure `values` is left untouched.
    ///
    /// # Panics
    ///
    /// If `values.len()` is not [`variables`](Self::variables). A generated
    /// caller passes a fixed-size array, so a mismatch is a codegen bug rather
    /// than anything a request can provoke — the same reasoning that makes
    /// [`format`](Self::format) panic.
    pub fn scan_into<'a>(&self, name: &'a str, values: &mut [&'a str]) -> Result<(), ParseError> {
        assert_eq!(
            values.len(),
            self.variables,
            "scan resource name against {:?}: wrong number of destinations",
            self.pattern
        );
        let error = |kind| ParseError {
            pattern: self.pattern.clone(),
            name: name.to_owned(),
            kind,
        };
        // Count first, so a length mismatch reports what the caller actually
        // sent rather than failing on whichever segment happens to differ.
        let got = name.split('/').count();
        if got != self.segments.len() {
            return Err(error(ParseErrorKind::SegmentCount {
                want: self.segments.len(),
                got,
            }));
        }
        // Two walks rather than one: the first rejects, the second assigns. A
        // single walk would leave the caller's destinations half-populated
        // when a later segment turns out not to match.
        for (index, (segment, part)) in self.segments.iter().zip(name.split('/')).enumerate() {
            match segment {
                Segment::Literal(literal) => {
                    if part != literal {
                        return Err(error(ParseErrorKind::Literal {
                            index,
                            want: literal.clone(),
                            got: part.to_owned(),
                        }));
                    }
                }
                Segment::Variable(variable) => {
                    if part.is_empty() {
                        return Err(error(ParseErrorKind::EmptyValue {
                            index,
                            name: variable.clone(),
                        }));
                    }
                }
            }
        }
        for (value, part) in values.iter_mut().zip(
            self.segments
                .iter()
                .zip(name.split('/'))
                .filter(|(segment, _)| matches!(segment, Segment::Variable(_)))
                .map(|(_, part)| part),
        ) {
            *value = part;
        }
        Ok(())
    }

    /// Matches the fully-qualified form of `name` — `//{domain}/` followed by
    /// the pattern — and returns its variable segments, in pattern order.
    ///
    /// `domain` is the service name, which by AIP-123 is the part of a
    /// resource type before the `/`: `example.com` for `example.com/Book`.
    ///
    /// The relative form is the one that travels in a `name` field; this is
    /// for the fully-qualified form that appears in cross-service references.
    pub fn scan_full<'a>(&self, name: &'a str, domain: &str) -> Result<Vec<&'a str>, ParseError> {
        let mut values = vec![""; self.variables];
        self.scan_full_into(name, domain, &mut values)?;
        Ok(values)
    }

    /// Matches the fully-qualified form of `name` and writes its variable
    /// segments into `values`, in pattern order.
    ///
    /// The allocation-free form of [`scan_full`](Self::scan_full); see
    /// [`scan_into`](Self::scan_into).
    ///
    /// # Panics
    ///
    /// If `values.len()` is not [`variables`](Self::variables); see
    /// [`scan_into`](Self::scan_into).
    pub fn scan_full_into<'a>(
        &self,
        name: &'a str,
        domain: &str,
        values: &mut [&'a str],
    ) -> Result<(), ParseError> {
        // Stripped a piece at a time, so a name that matches costs nothing;
        // only the error spells the prefix out.
        let relative = name
            .strip_prefix("//")
            .and_then(|rest| rest.strip_prefix(domain))
            .and_then(|rest| rest.strip_prefix('/'));
        let Some(relative) = relative else {
            // Reported against the name as given, since the prefix is what is
            // wrong with it; a failure past here reports the relative part it
            // actually scanned.
            return Err(ParseError {
                pattern: self.pattern.clone(),
                name: name.to_owned(),
                kind: ParseErrorKind::Prefix {
                    want: format!("//{domain}/"),
                },
            });
        };
        self.scan_into(relative, values)
    }

    /// Renders the pattern with `values` substituted for its variable
    /// segments, in pattern order.
    ///
    /// This does not validate the values — call [`validate`](Self::validate)
    /// for that.
    ///
    /// # Panics
    ///
    /// If `values.len()` is not [`variables`](Self::variables); see
    /// [`scan_into`](Self::scan_into).
    #[must_use]
    pub fn format<S: AsRef<str>>(&self, values: &[S]) -> String {
        assert_eq!(
            values.len(),
            self.variables,
            "format resource name against {:?}: wrong number of values",
            self.pattern
        );
        let size = self.segments.len() - 1
            + self
                .segments
                .iter()
                .map(|segment| match segment {
                    Segment::Literal(literal) => literal.len(),
                    Segment::Variable(_) => 0,
                })
                .sum::<usize>()
            + values
                .iter()
                .map(|value| value.as_ref().len())
                .sum::<usize>();
        let mut out = String::with_capacity(size);
        let mut next = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            if index > 0 {
                out.push('/');
            }
            match segment {
                Segment::Literal(literal) => out.push_str(literal),
                Segment::Variable(_) => {
                    out.push_str(values[next].as_ref());
                    next += 1;
                }
            }
        }
        out
    }

    /// Checks that `values` are usable as the variable segments of the
    /// pattern: each must be non-empty and free of the `/` separator.
    ///
    /// Errors name the offending variable segment, so a caller holding a
    /// parsed name learns which field is at fault.
    ///
    /// # Panics
    ///
    /// If `values.len()` is not [`variables`](Self::variables); see
    /// [`scan_into`](Self::scan_into).
    pub fn validate<S: AsRef<str>>(&self, values: &[S]) -> Result<(), InvalidResourceIdError> {
        assert_eq!(
            values.len(),
            self.variables,
            "validate resource name against {:?}: wrong number of values",
            self.pattern
        );
        for (name, value) in self.variable_names().zip(values) {
            if let Err(kind) = validate_resource_id(value.as_ref()) {
                return Err(InvalidResourceIdError {
                    segment: Some(name.to_owned()),
                    ..kind
                });
            }
        }
        Ok(())
    }

    /// Builds the error for a variable segment whose value the caller could not
    /// convert to the type that segment is declared to hold.
    ///
    /// Scanning only ever yields `&str`, so a segment with a stronger declared
    /// type — a UUID, say — is converted by the caller afterwards. This is how
    /// that conversion failing is reported as a name that does not match the
    /// pattern, rather than as a second error type alongside [`ParseError`]:
    ///
    /// ```
    /// use aip::ResourcePattern;
    ///
    /// let pattern: ResourcePattern = "books/{book}".parse()?;
    ///
    /// fn book_id(pattern: &ResourcePattern, name: &str) -> Result<u64, aip::resource::ParseError> {
    ///     let [scanned] = pattern.scan(name)?[..] else { unreachable!("one variable") };
    ///     scanned
    ///         .parse()
    ///         .map_err(|error| pattern.invalid_value(name, "book", error))
    /// }
    ///
    /// assert_eq!(book_id(&pattern, "books/17")?, 17);
    /// assert!(book_id(&pattern, "books/seventeen").is_err());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn invalid_value(
        &self,
        name: &str,
        variable: &str,
        reason: impl fmt::Display,
    ) -> ParseError {
        ParseError {
            pattern: self.pattern.clone(),
            name: name.to_owned(),
            kind: ParseErrorKind::InvalidValue {
                name: variable.to_owned(),
                reason: reason.to_string(),
            },
        }
    }

    /// Builds the error for a request field holding a bare id of this
    /// pattern's resource -- a create's `<resource>_id` (AIP-133) -- whose
    /// value is not one. It names the field rather than a segment, since the
    /// field is what the caller sent: `parse book_id "abc": <reason>`, as
    /// aip-go's generated `Parse<Resource>ID` reports it.
    ///
    /// ```
    /// use aip::ResourcePattern;
    ///
    /// let pattern: ResourcePattern = "books/{book}".parse()?;
    /// let error = pattern.invalid_id("book_id", "abc", "invalid length");
    /// assert_eq!(error.to_string(), r#"parse book_id "abc": invalid length"#);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn invalid_id(&self, field: &str, id: &str, reason: impl fmt::Display) -> ParseError {
        ParseError {
            pattern: self.pattern.clone(),
            name: id.to_owned(),
            kind: ParseErrorKind::InvalidId {
                field: field.to_owned(),
                reason: reason.to_string(),
            },
        }
    }
}

impl FromStr for ResourcePattern {
    type Err = CompileError;

    /// Compiles the pattern; see [`ResourcePattern::compile`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::compile(s)
    }
}

impl TryFrom<&str> for ResourcePattern {
    type Error = CompileError;

    /// Compiles the pattern; see [`ResourcePattern::compile`].
    fn try_from(pattern: &str) -> Result<Self, Self::Error> {
        Self::compile(pattern)
    }
}

/// Whether `text` contains a brace. Once a variable's single `{…}` wrapper is
/// stripped, any brace left means the segment is unbalanced or nested.
fn has_brace(text: &str) -> bool {
    text.contains(['{', '}'])
}

/// Whether `literal` is an AIP-122 collection identifier: `camelCase`, which is
/// a lowercase ASCII letter followed by ASCII letters and digits.
fn is_collection_id(literal: &str) -> bool {
    let mut chars = literal.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Whether `name` is usable as a variable name: `snake_case`, which is a
/// lowercase ASCII letter followed by lowercase letters, digits and `_`.
fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl fmt::Display for ResourcePattern {
    /// Writes the pattern this was compiled from.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

/// Checks that `id` is usable as a single resource ID segment.
///
/// An ID must be non-empty and must not contain the `/` separator, which would
/// silently split one segment into two.
pub fn validate_resource_id(id: &str) -> Result<(), InvalidResourceIdError> {
    let error = |kind| InvalidResourceIdError {
        segment: None,
        kind,
    };
    if id.is_empty() {
        return Err(error(InvalidResourceIdErrorKind::Empty));
    }
    if id.contains('/') {
        return Err(error(InvalidResourceIdErrorKind::Separator));
    }
    Ok(())
}

/// Checks that `id` is usable as the value of the variable segment `segment`.
///
/// [`validate_resource_id`] with the segment named, so the error says which
/// field is at fault. Generated code validates a name a segment at a time
/// rather than through [`ResourcePattern::validate`], because a segment whose
/// declared type is not a string has nothing to hand that function.
pub fn validate_segment(segment: &str, id: &str) -> Result<(), InvalidResourceIdError> {
    validate_resource_id(id).map_err(|error| InvalidResourceIdError {
        segment: Some(segment.to_owned()),
        ..error
    })
}

/// Returns whether any of `ids` is the AIP-159 [`WILDCARD`].
///
/// A name containing a wildcard identifies a collection to read across rather
/// than a single resource, so storage layers should treat it as a query rather
/// than a lookup.
pub fn contains_wildcard<S: AsRef<str>>(ids: &[S]) -> bool {
    ids.iter().any(|id| id.as_ref() == WILDCARD)
}

/// The behaviour shared by every resource name type emitted by
/// `protoc-gen-rust-aip`.
///
/// It exists for code that must handle a resource name without knowing which
/// one it is — middleware that logs, authorizes or audits whatever `name` a
/// request carries. Code that knows the type should use it directly: a
/// `BookName` has a typed `parent()`, which this trait deliberately cannot
/// express.
///
/// Unlike the Go interface this replaces, which generated types satisfied
/// structurally, generated code must write out `impl ResourceName for BookName`
/// and so must import this crate.
pub trait ResourceName: fmt::Display {
    /// The AIP resource type, e.g. `example.com/Book`.
    fn resource_type(&self) -> &str;

    /// The resource name pattern, e.g. `publishers/{publisher}/books/{book}`.
    fn pattern(&self) -> &str;

    /// Whether every variable segment is usable.
    fn validate(&self) -> Result<(), InvalidResourceIdError>;

    /// Whether any variable segment is the AIP-159 [`WILDCARD`], meaning the
    /// name identifies a collection to read across rather than a single
    /// resource.
    fn contains_wildcard(&self) -> bool;

    /// The fully-qualified name, e.g.
    /// `//example.com/publishers/p1/books/b1`.
    ///
    /// The default implementation takes the service domain from
    /// [`resource_type`](Self::resource_type), which by AIP-123 is the part
    /// before the `/`.
    fn full_name(&self) -> String {
        let resource_type = self.resource_type();
        let domain = resource_type
            .split_once('/')
            .map_or(resource_type, |(domain, _)| domain);
        format!("//{domain}/{self}")
    }
}

/// A message that declares `google.api.resource` (AIP-123): what a resource
/// adds to the [`FieldBehavior`] every generated message has.
///
/// Implemented by `protoc-gen-rust-aip` for every resource message: it reads
/// the field mask an update of it implies, and clears the paths of a kind from
/// a mask an update sends. The `Default` it requires is the one every generated
/// message has. Its name -- the type, pattern, validation and formatting -- is
/// the generated [`ResourceName`] type, which the resource's generated
/// `parse_name` returns.
///
/// ```ignore
/// if update_mask.paths.is_empty() {
///     item.clear_fields(FieldBehaviorKind::OutputOnly);
///     update_mask = item.implied_field_mask();
/// } else {
///     item.clear_field_mask(FieldBehaviorKind::OutputOnly, &mut update_mask);
/// }
/// ```
pub trait Resource: FieldBehavior + Default {
    /// The field mask AIP-134 implies when an update omits one: every field an
    /// update may write — not `OUTPUT_ONLY`, `IDENTIFIER` or `IMMUTABLE` — that
    /// this value sets, in declaration order.
    ///
    /// Set is protobuf presence: a field with explicit presence — `optional`, a
    /// message, a oneof member — when present, even at its zero value; any
    /// other scalar when not its zero value; a repeated field or a map when not
    /// empty. So a zero value without explicit presence is never implied: a
    /// client clears one with an explicit mask, or the schema makes the field
    /// `optional`.
    ///
    /// A set message field is implied whole — `carrier`, not `carrier.name` —
    /// so the update writes everything beneath it, nested `OUTPUT_ONLY` values
    /// included. Clear, then imply: clear `OUTPUT_ONLY` fields first.
    ///
    /// See <https://google.aip.dev/134#field-masks>.
    #[must_use]
    fn implied_field_mask(&self) -> FieldMask;

    /// Removes from `mask` the paths that name, or pass through, a field
    /// annotated `kind`, keeping the rest in the order `mask` holds them. It
    /// edits `mask` in place, as [`clear_fields`](FieldBehavior::clear_fields)
    /// edits a message, and reads only the schema of `Self`, never the values
    /// of `self`.
    ///
    /// A path is walked segment by segment, through singular message fields
    /// and oneof members, so `carrier.tracking_id` is removed when either
    /// `carrier` or `tracking_id` is annotated `kind`; `carrier` is kept when
    /// only fields beneath it are. A path that names no field, such as `*`, is
    /// kept as written: whether it applies is the mask's own validation to
    /// decide.
    ///
    /// AIP-203 has a server ignore the `OUTPUT_ONLY` fields an update mask
    /// names rather than reject the update:
    ///
    /// ```ignore
    /// if let Some(mask) = request.update_mask.as_option_mut() {
    ///     request.book.clear_field_mask(FieldBehaviorKind::OutputOnly, mask);
    /// }
    /// ```
    ///
    /// Built on [`validate_fields`](FieldBehavior::validate_fields), run on
    /// `Self::default()`, where every field is unset: a path is removed when
    /// it, or a path above it, names a field the check reports. Not generated.
    ///
    /// See <https://google.aip.dev/203#output-only>.
    fn clear_field_mask(&self, kind: FieldBehaviorKind, mask: &mut FieldMask) {
        let schema = Self::default();
        mask.paths.retain(|path| !names_kind(&schema, kind, path));
    }
}

/// Whether `path`, or a path above it, names a field of `schema` annotated
/// `kind`. `schema` is a default message, so every field is unset, and
/// [`FieldBehavior::validate_fields`] reports a field of `kind` exactly when
/// the scope names it.
fn names_kind<M: FieldBehavior>(schema: &M, kind: FieldBehaviorKind, path: &str) -> bool {
    path.match_indices('.')
        .map(|(end, _)| &path[..end])
        .chain([path])
        .any(|prefix| {
            let scope = [prefix.to_owned()];
            schema
                .validate_fields(kind, FieldScope::Mask(&scope))
                .is_err_and(|error| error.paths.iter().any(|field| field.to_string() == prefix))
        })
}

/// Why a resource name pattern could not be compiled.
///
/// A bad pattern is a codegen or programming error, not anything a request can
/// provoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pattern: String,
    kind: CompileErrorKind,
}

impl CompileError {
    /// Returns the pattern that failed to compile.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns what was wrong with it.
    #[must_use]
    pub fn kind(&self) -> &CompileErrorKind {
        &self.kind
    }
}

/// What was wrong with a resource name pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompileErrorKind {
    /// The pattern was empty.
    EmptyPattern,
    /// A segment with no text, from a leading, trailing or doubled `/`.
    EmptySegment(usize),
    /// A segment with unbalanced or nested braces.
    MalformedSegment(String),
    /// A `{}` with no variable name inside.
    EmptyVariableName(usize),
    /// A variable name used by more than one segment.
    DuplicateVariable(String),
    /// A literal segment that is not a `camelCase` collection identifier.
    InvalidLiteral(String),
    /// A variable name that is not `snake_case`.
    InvalidVariableName(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "compile resource pattern {:?}: ", self.pattern)?;
        match &self.kind {
            CompileErrorKind::EmptyPattern => f.write_str("empty pattern"),
            CompileErrorKind::EmptySegment(index) => write!(f, "empty segment {index}"),
            CompileErrorKind::MalformedSegment(segment) => {
                write!(f, "malformed segment {segment:?}")
            }
            CompileErrorKind::EmptyVariableName(index) => {
                write!(f, "empty variable name in segment {index}")
            }
            CompileErrorKind::DuplicateVariable(name) => {
                write!(f, "duplicate variable {name:?}")
            }
            CompileErrorKind::InvalidLiteral(literal) => {
                write!(
                    f,
                    "literal {literal:?} is not a camelCase collection identifier"
                )
            }
            CompileErrorKind::InvalidVariableName(name) => {
                write!(f, "variable name {name:?} is not snake_case")
            }
        }
    }
}

impl core::error::Error for CompileError {}

/// Why a string does not parse as a resource name of a pattern -- what every
/// generated `parse` returns, what a [`FieldError`](crate::FieldError) from
/// `parse_name` and `parse_<field>` says is wrong, and
/// [`ResourcePattern::scan`] underneath them.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pattern: String,
    name: String,
    kind: ParseErrorKind,
}

impl ParseError {
    /// Returns the pattern the name was parsed against.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns the name that failed to parse, or for
    /// [`ParseErrorKind::InvalidId`] the id.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns what was wrong with it.
    #[must_use]
    pub fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }
}

/// What was wrong with a resource name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The name has the wrong number of `/`-separated segments.
    SegmentCount {
        /// The number the pattern has.
        want: usize,
        /// The number the name has.
        got: usize,
    },
    /// A literal segment that does not match.
    Literal {
        /// The segment's position in the pattern.
        index: usize,
        /// The literal the pattern requires.
        want: String,
        /// What the name has instead.
        got: String,
    },
    /// A variable segment with no value.
    EmptyValue {
        /// The segment's position in the pattern.
        index: usize,
        /// The variable's name.
        name: String,
    },
    /// A fully-qualified name that does not start with `//{domain}/`.
    ///
    /// Only [`scan_full`](ResourcePattern::scan_full) produces this; the
    /// relative form has no prefix to get wrong.
    Prefix {
        /// The prefix the service requires, e.g. `//example.com/`.
        want: String,
    },
    /// A variable segment whose value is not valid for the type that segment is
    /// declared to hold.
    ///
    /// Raised by the caller through
    /// [`invalid_value`](ResourcePattern::invalid_value), not by the scan
    /// itself, which has no opinion about anything beyond the separator.
    InvalidValue {
        /// The variable's name.
        name: String,
        /// What the conversion said was wrong with it.
        reason: String,
    },
    /// A request field holding a bare id, not a name -- a create's
    /// `<resource>_id` -- whose value is not one. [`ParseError::name`] is the
    /// id.
    ///
    /// Raised by the caller through
    /// [`invalid_id`](ResourcePattern::invalid_id).
    InvalidId {
        /// The request field, e.g. `book_id`.
        field: String,
        /// What was wrong with the id.
        reason: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A bare id is not a name, so it is not parsed against the pattern.
        if let ParseErrorKind::InvalidId { field, reason } = &self.kind {
            return write!(f, "parse {field} {:?}: {reason}", self.name);
        }
        write!(
            f,
            "invalid resource name: parse {:?} against {:?}: ",
            self.name, self.pattern
        )?;
        match &self.kind {
            ParseErrorKind::SegmentCount { want, got } => {
                write!(f, "bad number of segments, want {want}, got {got}")
            }
            ParseErrorKind::Literal { index, want, got } => {
                write!(f, "bad segment {index}, want {want:?}, got {got:?}")
            }
            ParseErrorKind::EmptyValue { index, name } => {
                write!(f, "empty value for segment {index} ({name})")
            }
            ParseErrorKind::Prefix { want } => {
                write!(f, "bad prefix, want {want:?}")
            }
            ParseErrorKind::InvalidValue { name, reason } => {
                write!(f, "invalid value for segment ({name}): {reason}")
            }
            ParseErrorKind::InvalidId { .. } => unreachable!("written above"),
        }
    }
}

impl core::error::Error for ParseError {}

/// Why a name matched none of the patterns declared for a resource.
///
/// A resource with more than one `pattern` is scanned against each in
/// declaration order; this collects what went wrong with every one of them, so
/// the caller can say more than "no". Map it to `InvalidArgument` at the RPC
/// boundary.
///
/// Generated code constructs this, which is why [`new`](Self::new) is public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoPatternError {
    name: String,
    attempts: Vec<ParseError>,
}

impl NoPatternError {
    /// Collects the failures from scanning `name` against every pattern.
    #[must_use]
    pub fn new(name: impl Into<String>, attempts: Vec<ParseError>) -> Self {
        Self {
            name: name.into(),
            attempts,
        }
    }

    /// Returns the name that matched nothing.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns why each pattern rejected it, in declaration order.
    #[must_use]
    pub fn attempts(&self) -> &[ParseError] {
        &self.attempts
    }
}

impl fmt::Display for NoPatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid resource name: {:?} matches none of the patterns:",
            self.name
        )?;
        // The pattern alone, not each attempt's full message: every one of them
        // repeats the same name, and the reason a non-matching pattern was
        // rejected is rarely the interesting part.
        for attempt in &self.attempts {
            write!(f, " {:?}", attempt.pattern())?;
        }
        Ok(())
    }
}

impl core::error::Error for NoPatternError {}

/// Why a string is not usable as a resource ID segment.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidResourceIdError {
    /// The variable segment at fault, when the ID came from a pattern.
    segment: Option<String>,
    kind: InvalidResourceIdErrorKind,
}

impl InvalidResourceIdError {
    /// The error for a variable segment with no value.
    ///
    /// A segment whose declared type is not a string has its own idea of what
    /// "no value" is — the nil UUID, say — which only the code holding that
    /// type can recognise, so this is what it reports with.
    #[must_use]
    pub fn empty(segment: &str) -> Self {
        Self {
            segment: Some(segment.to_owned()),
            kind: InvalidResourceIdErrorKind::Empty,
        }
    }

    /// Returns the name of the variable segment at fault, if the ID was
    /// checked as part of a pattern.
    #[must_use]
    pub fn segment(&self) -> Option<&str> {
        self.segment.as_deref()
    }

    /// Returns what was wrong with the ID.
    #[must_use]
    pub fn kind(&self) -> &InvalidResourceIdErrorKind {
        &self.kind
    }
}

/// What was wrong with a resource ID.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidResourceIdErrorKind {
    /// The ID was empty.
    Empty,
    /// The ID contains the `/` separator, which would split one segment into
    /// two.
    Separator,
}

impl fmt::Display for InvalidResourceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(segment) = &self.segment {
            write!(f, "{segment}: ")?;
        }
        match self.kind {
            InvalidResourceIdErrorKind::Empty => f.write_str("empty"),
            InvalidResourceIdErrorKind::Separator => f.write_str("contains illegal character '/'"),
        }
    }
}

impl core::error::Error for InvalidResourceIdError {}

#[cfg(test)]
mod tests;
