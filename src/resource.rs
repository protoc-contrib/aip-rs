//! AIP-122 resource names: compiling a name pattern once, then scanning and
//! formatting against it.
//!
//! See <https://google.aip.dev/122>.

use core::fmt;
use core::str::FromStr;

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
/// # Ok::<_, aip::resource::ScanError>(())
/// ```
///
/// A compiled pattern is immutable and safe to share across threads.
///
/// [`LazyLock`]: std::sync::LazyLock
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePattern {
    pattern: String,
    segments: Vec<Segment>,
    variables: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    /// The literal text for a literal segment, or the variable name (without
    /// braces) for a variable segment.
    name: String,
    /// Whether this segment captures a value.
    variable: bool,
}

impl ResourcePattern {
    /// Compiles a resource name pattern.
    ///
    /// A segment written as `{name}` captures a value; every other segment is
    /// a literal that must match exactly. Variable names must be unique so
    /// that error messages can identify a segment unambiguously.
    ///
    /// [`str::parse`] is the same thing and usually reads better.
    pub fn compile(pattern: &str) -> Result<Self, CompileError> {
        let error = |kind| CompileError {
            pattern: pattern.to_owned(),
            kind,
        };
        if pattern.is_empty() {
            return Err(error(CompileErrorKind::EmptyPattern));
        }
        let parts: Vec<&str> = pattern.split('/').collect();
        let mut segments = Vec::with_capacity(parts.len());
        let mut variables = 0;
        for (index, part) in parts.into_iter().enumerate() {
            let Some(name) = part.strip_prefix('{') else {
                if part.is_empty() {
                    return Err(error(CompileErrorKind::EmptySegment(index)));
                }
                if part.contains(['{', '}']) {
                    return Err(error(CompileErrorKind::MalformedSegment(part.to_owned())));
                }
                segments.push(Segment {
                    name: part.to_owned(),
                    variable: false,
                });
                continue;
            };
            let Some(name) = name.strip_suffix('}') else {
                return Err(error(CompileErrorKind::MalformedSegment(part.to_owned())));
            };
            if name.is_empty() {
                return Err(error(CompileErrorKind::EmptyVariableName(index)));
            }
            if name.contains(['{', '}']) {
                return Err(error(CompileErrorKind::MalformedSegment(part.to_owned())));
            }
            if segments.iter().any(|s| s.variable && s.name == name) {
                return Err(error(CompileErrorKind::DuplicateVariable(name.to_owned())));
            }
            segments.push(Segment {
                name: name.to_owned(),
                variable: true,
            });
            variables += 1;
        }
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
        self.segments
            .iter()
            .filter(|segment| segment.variable)
            .map(|segment| segment.name.as_str())
    }

    /// Matches `name` against the pattern and returns its variable segments,
    /// in pattern order.
    ///
    /// The values borrow from `name`. Literal segments must match exactly and
    /// variable segments must be non-empty.
    pub fn scan<'a>(&self, name: &'a str) -> Result<Vec<&'a str>, ScanError> {
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
    pub fn scan_into<'a>(&self, name: &'a str, values: &mut [&'a str]) -> Result<(), ScanError> {
        assert_eq!(
            values.len(),
            self.variables,
            "scan resource name against {:?}: wrong number of destinations",
            self.pattern
        );
        let error = |kind| ScanError {
            pattern: self.pattern.clone(),
            name: name.to_owned(),
            kind,
        };
        // Count first, so a length mismatch reports what the caller actually
        // sent rather than failing on whichever segment happens to differ.
        let got = name.split('/').count();
        if got != self.segments.len() {
            return Err(error(ScanErrorKind::SegmentCount {
                want: self.segments.len(),
                got,
            }));
        }
        // Two walks rather than one: the first rejects, the second assigns. A
        // single walk would leave the caller's destinations half-populated
        // when a later segment turns out not to match.
        for (index, (segment, part)) in self.segments.iter().zip(name.split('/')).enumerate() {
            if segment.variable {
                if part.is_empty() {
                    return Err(error(ScanErrorKind::EmptyValue {
                        index,
                        name: segment.name.clone(),
                    }));
                }
            } else if part != segment.name {
                return Err(error(ScanErrorKind::Literal {
                    index,
                    want: segment.name.clone(),
                    got: part.to_owned(),
                }));
            }
        }
        for (value, part) in values.iter_mut().zip(
            self.segments
                .iter()
                .zip(name.split('/'))
                .filter(|(segment, _)| segment.variable)
                .map(|(_, part)| part),
        ) {
            *value = part;
        }
        Ok(())
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
                .filter(|segment| !segment.variable)
                .map(|segment| segment.name.len())
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
            if segment.variable {
                out.push_str(values[next].as_ref());
                next += 1;
            } else {
                out.push_str(&segment.name);
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
}

impl FromStr for ResourcePattern {
    type Err = CompileError;

    /// Compiles the pattern; see [`ResourcePattern::compile`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::compile(s)
    }
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
    if id.is_empty() {
        return Err(InvalidResourceIdError {
            segment: None,
            empty: true,
        });
    }
    if id.contains('/') {
        return Err(InvalidResourceIdError {
            segment: None,
            empty: false,
        });
    }
    Ok(())
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
        }
    }
}

impl core::error::Error for CompileError {}

/// Why a string is not a resource name of the pattern it was scanned against.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pattern: String,
    name: String,
    kind: ScanErrorKind,
}

impl ScanError {
    /// Returns the pattern the name was scanned against.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns the name that failed to scan.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns what was wrong with it.
    #[must_use]
    pub fn kind(&self) -> &ScanErrorKind {
        &self.kind
    }
}

/// What was wrong with a resource name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanErrorKind {
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
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid resource name: parse {:?} against {:?}: ",
            self.name, self.pattern
        )?;
        match &self.kind {
            ScanErrorKind::SegmentCount { want, got } => {
                write!(f, "bad number of segments, want {want}, got {got}")
            }
            ScanErrorKind::Literal { index, want, got } => {
                write!(f, "bad segment {index}, want {want:?}, got {got:?}")
            }
            ScanErrorKind::EmptyValue { index, name } => {
                write!(f, "empty value for segment {index} ({name})")
            }
        }
    }
}

impl core::error::Error for ScanError {}

/// Why a string is not usable as a resource ID segment.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidResourceIdError {
    /// The variable segment at fault, when the ID came from a pattern.
    segment: Option<String>,
    /// Whether the ID was empty, as opposed to containing a `/`.
    empty: bool,
}

impl InvalidResourceIdError {
    /// Returns the name of the variable segment at fault, if the ID was
    /// checked as part of a pattern.
    #[must_use]
    pub fn segment(&self) -> Option<&str> {
        self.segment.as_deref()
    }
}

impl fmt::Display for InvalidResourceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(segment) = &self.segment {
            write!(f, "{segment}: ")?;
        }
        if self.empty {
            f.write_str("empty")
        } else {
            f.write_str("contains illegal character '/'")
        }
    }
}

impl core::error::Error for InvalidResourceIdError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> ResourcePattern {
        ResourcePattern::compile("publishers/{publisher}/books/{book}").unwrap()
    }

    #[test]
    fn compiles_and_reports_its_shape() {
        let pattern = book();
        assert_eq!(pattern.as_str(), "publishers/{publisher}/books/{book}");
        assert_eq!(pattern.to_string(), "publishers/{publisher}/books/{book}");
        assert_eq!(pattern.variables(), 2);
        assert_eq!(
            pattern.variable_names().collect::<Vec<_>>(),
            ["publisher", "book"]
        );
    }

    #[test]
    fn compiles_a_pattern_with_no_variables() {
        let pattern = ResourcePattern::compile("publishers").unwrap();
        assert_eq!(pattern.variables(), 0);
        assert_eq!(pattern.scan("publishers"), Ok(Vec::new()));
        assert_eq!(pattern.format::<&str>(&[]), "publishers");
    }

    #[test]
    fn rejects_malformed_patterns() {
        let kind = |pattern: &str| ResourcePattern::compile(pattern).unwrap_err().kind;
        assert_eq!(kind(""), CompileErrorKind::EmptyPattern);
        assert_eq!(kind("publishers//books"), CompileErrorKind::EmptySegment(1));
        assert_eq!(
            kind("publishers/{publisher"),
            CompileErrorKind::MalformedSegment("{publisher".to_owned())
        );
        assert_eq!(
            kind("publishers/publisher}"),
            CompileErrorKind::MalformedSegment("publisher}".to_owned())
        );
        assert_eq!(
            kind("publishers/{{publisher}}"),
            CompileErrorKind::MalformedSegment("{{publisher}}".to_owned())
        );
        assert_eq!(
            kind("publishers/{}"),
            CompileErrorKind::EmptyVariableName(1)
        );
        assert_eq!(
            kind("books/{book}/editions/{book}"),
            CompileErrorKind::DuplicateVariable("book".to_owned())
        );
    }

    #[test]
    fn scans_a_matching_name() {
        assert_eq!(book().scan("publishers/p1/books/b1"), Ok(vec!["p1", "b1"]));
    }

    #[test]
    fn scans_without_allocating() {
        let mut ids = [""; 2];
        book()
            .scan_into("publishers/p1/books/b1", &mut ids)
            .unwrap();
        assert_eq!(ids, ["p1", "b1"]);
    }

    #[test]
    fn leaves_destinations_untouched_on_failure() {
        let mut ids = ["untouched"; 2];
        assert!(
            book()
                .scan_into("publishers/p1/shelves/s1", &mut ids)
                .is_err()
        );
        assert_eq!(ids, ["untouched", "untouched"]);
    }

    #[test]
    fn rejects_names_that_do_not_match() {
        let kind = |name: &str| book().scan(name).unwrap_err().kind;
        assert_eq!(
            kind("publishers/p1/books"),
            ScanErrorKind::SegmentCount { want: 4, got: 3 }
        );
        assert_eq!(
            kind("publishers/p1/books/b1/pages/g1"),
            ScanErrorKind::SegmentCount { want: 4, got: 6 }
        );
        assert_eq!(
            kind("publishers/p1/shelves/s1"),
            ScanErrorKind::Literal {
                index: 2,
                want: "books".to_owned(),
                got: "shelves".to_owned(),
            }
        );
        assert_eq!(
            kind("publishers//books/b1"),
            ScanErrorKind::EmptyValue {
                index: 1,
                name: "publisher".to_owned(),
            }
        );
    }

    #[test]
    fn error_reports_the_offending_name() {
        let error = book().scan("publishers/p1/shelves/s1").unwrap_err();
        assert_eq!(error.name(), "publishers/p1/shelves/s1");
        assert_eq!(error.pattern(), "publishers/{publisher}/books/{book}");
        assert_eq!(
            error.to_string(),
            r#"invalid resource name: parse "publishers/p1/shelves/s1" against "publishers/{publisher}/books/{book}": bad segment 2, want "books", got "shelves""#
        );
    }

    #[test]
    fn formats_and_round_trips() {
        let pattern = book();
        let name = pattern.format(&["p1", "b1"]);
        assert_eq!(name, "publishers/p1/books/b1");
        assert_eq!(pattern.scan(&name), Ok(vec!["p1", "b1"]));
    }

    #[test]
    #[should_panic(expected = "wrong number of values")]
    fn format_panics_on_the_wrong_arity() {
        let _ = book().format(&["p1"]);
    }

    #[test]
    #[should_panic(expected = "wrong number of destinations")]
    fn scan_into_panics_on_the_wrong_arity() {
        let mut ids = [""; 1];
        let _ = book().scan_into("publishers/p1/books/b1", &mut ids);
    }

    #[test]
    fn validates_resource_ids() {
        assert!(validate_resource_id("b1").is_ok());
        assert_eq!(validate_resource_id("").unwrap_err().to_string(), "empty");
        assert_eq!(
            validate_resource_id("a/b").unwrap_err().to_string(),
            "contains illegal character '/'"
        );
        // The wildcard is a legal ID; it is meaning, not syntax.
        assert!(validate_resource_id(WILDCARD).is_ok());
    }

    #[test]
    fn validate_names_the_offending_segment() {
        let error = book().validate(&["p1", "b/1"]).unwrap_err();
        assert_eq!(error.segment(), Some("book"));
        assert_eq!(error.to_string(), "book: contains illegal character '/'");
        assert!(book().validate(&["p1", "b1"]).is_ok());
    }

    #[test]
    fn detects_the_wildcard() {
        assert!(contains_wildcard(&["p1", "-"]));
        assert!(!contains_wildcard(&["p1", "b1"]));
        assert!(!contains_wildcard::<&str>(&[]));
    }

    struct BookName {
        publisher: String,
        book: String,
    }

    impl fmt::Display for BookName {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "publishers/{}/books/{}", self.publisher, self.book)
        }
    }

    impl ResourceName for BookName {
        fn resource_type(&self) -> &str {
            "example.com/Book"
        }

        fn pattern(&self) -> &str {
            "publishers/{publisher}/books/{book}"
        }

        fn validate(&self) -> Result<(), InvalidResourceIdError> {
            book().validate(&[&self.publisher, &self.book])
        }

        fn contains_wildcard(&self) -> bool {
            contains_wildcard(&[&self.publisher, &self.book])
        }
    }

    #[test]
    fn resource_name_derives_the_full_name_from_the_type() {
        let name = BookName {
            publisher: "p1".to_owned(),
            book: "b1".to_owned(),
        };
        assert_eq!(name.to_string(), "publishers/p1/books/b1");
        assert_eq!(name.full_name(), "//example.com/publishers/p1/books/b1");
        assert!(name.validate().is_ok());
        assert!(!name.contains_wildcard());
    }
}
