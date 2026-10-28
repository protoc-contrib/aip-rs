use super::*;
use crate::{FieldBehaviorError, FieldPath, FieldType};

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
    assert_eq!(
        ResourcePattern::try_from("publishers/{publisher}/books/{book}"),
        Ok(pattern)
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
fn holds_segments_to_aip_122_spelling() {
    let kind = |pattern: &str| ResourcePattern::compile(pattern).unwrap_err().kind;
    for literal in [
        "Publishers",
        "book_shelves",
        "book-shelves",
        "a b",
        "1books",
    ] {
        assert_eq!(
            kind(&format!("{literal}/{{id}}")),
            CompileErrorKind::InvalidLiteral(literal.to_owned())
        );
    }
    for variable in ["book=**", "pub lisher", "billingAccount", "_book", "1book"] {
        assert_eq!(
            kind(&format!("books/{{{variable}}}")),
            CompileErrorKind::InvalidVariableName(variable.to_owned())
        );
    }
    let pattern = ResourcePattern::compile("billingAccounts/{billing_account}/logs/{log2}");
    assert!(pattern.is_ok());
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
        ParseErrorKind::SegmentCount { want: 4, got: 3 }
    );
    assert_eq!(
        kind("publishers/p1/books/b1/pages/g1"),
        ParseErrorKind::SegmentCount { want: 4, got: 6 }
    );
    assert_eq!(
        kind("publishers/p1/shelves/s1"),
        ParseErrorKind::Literal {
            index: 2,
            want: "books".to_owned(),
            got: "shelves".to_owned(),
        }
    );
    assert_eq!(
        kind("publishers//books/b1"),
        ParseErrorKind::EmptyValue {
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
fn scans_a_fully_qualified_name() {
    assert_eq!(
        book().scan_full("//example.com/publishers/p1/books/b1", "example.com"),
        Ok(vec!["p1", "b1"])
    );
}

#[test]
fn rejects_a_fully_qualified_name_with_the_wrong_prefix() {
    let error = book()
        .scan_full("//other.example/publishers/p1/books/b1", "example.com")
        .unwrap_err();
    assert_eq!(
        error.kind(),
        &ParseErrorKind::Prefix {
            want: "//example.com/".to_owned(),
        }
    );
    // The prefix is what is wrong, so the whole name is what is reported.
    assert_eq!(error.name(), "//other.example/publishers/p1/books/b1");
    assert_eq!(
        error.to_string(),
        r#"invalid resource name: parse "//other.example/publishers/p1/books/b1" against "publishers/{publisher}/books/{book}": bad prefix, want "//example.com/""#
    );
}

#[test]
fn a_relative_name_is_not_a_fully_qualified_one() {
    let error = book()
        .scan_full("publishers/p1/books/b1", "example.com")
        .unwrap_err();
    assert!(matches!(error.kind(), ParseErrorKind::Prefix { .. }));
}

#[test]
fn a_domain_must_end_at_the_separator() {
    let error = book()
        .scan_full("//example.comx/publishers/p1/books/b1", "example.com")
        .unwrap_err();
    assert!(matches!(error.kind(), ParseErrorKind::Prefix { .. }));
}

#[test]
fn past_the_prefix_a_full_scan_reports_the_relative_name() {
    let error = book()
        .scan_full("//example.com/publishers/p1/shelves/s1", "example.com")
        .unwrap_err();
    assert_eq!(error.name(), "publishers/p1/shelves/s1");
}

#[test]
fn no_pattern_error_lists_what_it_tried() {
    let author_book = ResourcePattern::compile("authors/{author}/books/{book}").unwrap();
    let name = "shelves/s1/books/b1";
    let attempts = vec![
        book().scan(name).unwrap_err(),
        author_book.scan(name).unwrap_err(),
    ];
    let error = NoPatternError::new(name, attempts);
    assert_eq!(error.name(), name);
    assert_eq!(error.attempts().len(), 2);
    assert_eq!(
        error.to_string(),
        r#"invalid resource name: "shelves/s1/books/b1" matches none of the patterns: "publishers/{publisher}/books/{book}" "authors/{author}/books/{book}""#
    );
}

#[test]
fn reports_a_value_its_segment_type_rejects() {
    let pattern = book();
    let error = pattern.invalid_value(
        "publishers/p1/books/not-a-uuid",
        "book",
        "invalid length: expected 36, found 10",
    );
    assert_eq!(
        error.kind(),
        &ParseErrorKind::InvalidValue {
            name: "book".to_owned(),
            reason: "invalid length: expected 36, found 10".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        r#"invalid resource name: parse "publishers/p1/books/not-a-uuid" against "publishers/{publisher}/books/{book}": invalid value for segment (book): invalid length: expected 36, found 10"#
    );
}

#[test]
fn reports_an_id_field_by_its_name() {
    let error = book().invalid_id("book_id", "abc", "invalid length: expected 36, found 3");
    assert_eq!(
        error.kind(),
        &ParseErrorKind::InvalidId {
            field: "book_id".to_owned(),
            reason: "invalid length: expected 36, found 3".to_owned(),
        }
    );
    assert_eq!(error.name(), "abc");
    assert_eq!(
        error.to_string(),
        r#"parse book_id "abc": invalid length: expected 36, found 3"#
    );
}

#[test]
fn validates_one_segment_at_a_time() {
    assert!(validate_segment("book", "b1").is_ok());
    let error = validate_segment("book", "b/1").unwrap_err();
    assert_eq!(error.segment(), Some("book"));
    assert_eq!(error.kind(), &InvalidResourceIdErrorKind::Separator);
    assert_eq!(error.to_string(), "book: contains illegal character '/'");
}

#[test]
fn a_segment_can_report_itself_empty() {
    let error = InvalidResourceIdError::empty("book");
    assert_eq!(error.segment(), Some("book"));
    assert_eq!(error.kind(), &InvalidResourceIdErrorKind::Empty);
    assert_eq!(error.to_string(), "book: empty");
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

/// A carrier, with an unannotated `name`, left out, and an `OUTPUT_ONLY`
/// `tracking_id` (2).
#[derive(Default)]
struct Carrier {
    tracking_id: String,
}

impl FieldBehavior for Carrier {
    fn validate_fields(
        &self,
        kind: FieldBehaviorKind,
        scope: FieldScope<'_>,
    ) -> Result<(), FieldBehaviorError> {
        let mut paths = Vec::new();
        if kind == FieldBehaviorKind::OutputOnly
            && scope.contains("tracking_id")
            && self.tracking_id.is_empty()
        {
            paths.push(FieldPath::field("tracking_id", 2, FieldType::String));
        }
        FieldBehaviorError::check(kind, paths)
    }
}

/// A shipment, with an `OUTPUT_ONLY` `create_time` (4), a `carrier` (7)
/// and an `IMMUTABLE` `origin_carrier` (13), checked as
/// protoc-gen-rust-aip generates the check: a message field the scope
/// names is entered when set, one it names fields beneath is entered as
/// held, or as its default when unset. Its unannotated scalars, `origin`
/// and `notes`, take no part in the check, so they are left out.
#[derive(Default)]
struct Shipment {
    create_time: Option<u64>,
    carrier: Option<Carrier>,
    origin_carrier: Option<Carrier>,
}

impl FieldBehavior for Shipment {
    fn validate_fields(
        &self,
        kind: FieldBehaviorKind,
        scope: FieldScope<'_>,
    ) -> Result<(), FieldBehaviorError> {
        let mut paths = Vec::new();
        if kind == FieldBehaviorKind::OutputOnly
            && scope.contains("create_time")
            && self.create_time.is_none()
        {
            paths.push(FieldPath::field("create_time", 4, FieldType::Message));
        }
        if kind == FieldBehaviorKind::Immutable
            && scope.contains("origin_carrier")
            && self.origin_carrier.is_none()
        {
            paths.push(FieldPath::field("origin_carrier", 13, FieldType::Message));
        }
        if kind == FieldBehaviorKind::OutputOnly {
            for (name, number, carrier) in [
                ("carrier", 7, &self.carrier),
                ("origin_carrier", 13, &self.origin_carrier),
            ] {
                let path = FieldPath::field(name, number, FieldType::Message);
                let checked = if scope.contains(name) {
                    carrier
                        .as_ref()
                        .map(|carrier| carrier.validate_fields(kind, FieldScope::All))
                } else {
                    let FieldScope::Mask(mask) = scope else {
                        unreachable!("All contains every field")
                    };
                    let beneath: Vec<String> = mask
                        .iter()
                        .filter_map(|path| path.strip_prefix(name)?.strip_prefix('.'))
                        .map(ToOwned::to_owned)
                        .collect();
                    (!beneath.is_empty()).then(|| {
                        carrier
                            .as_ref()
                            .unwrap_or(&Carrier::default())
                            .validate_fields(kind, FieldScope::Mask(&beneath))
                    })
                };
                if let Some(Err(error)) = checked {
                    paths.extend(error.prefixed(&path).paths);
                }
            }
        }
        FieldBehaviorError::check(kind, paths)
    }
}

fn mask(paths: &[&str]) -> FieldMask {
    FieldMask {
        paths: paths.iter().map(|&path| path.to_owned()).collect(),
        ..Default::default()
    }
}

impl Resource for Shipment {
    fn implied_field_mask(&self) -> FieldMask {
        FieldMask::default()
    }
}

fn cleared<M: Resource>(message: &M, kind: FieldBehaviorKind, paths: &[&str]) -> Vec<String> {
    let mut mask = mask(paths);
    message.clear_field_mask(kind, &mut mask);
    mask.paths
}

#[test]
fn clear_field_mask_removes_the_paths_of_the_kind_keeping_the_rest_in_order() {
    assert_eq!(
        cleared(
            &Shipment::default(),
            FieldBehaviorKind::OutputOnly,
            &["notes", "create_time", "origin"]
        ),
        ["notes", "origin"]
    );
}

#[test]
fn clear_field_mask_removes_a_nested_path_through_a_message_to_the_kind() {
    assert_eq!(
        cleared(
            &Shipment::default(),
            FieldBehaviorKind::OutputOnly,
            &["carrier.tracking_id", "carrier.name"]
        ),
        ["carrier.name"]
    );
}

#[test]
fn clear_field_mask_removes_a_path_beneath_a_field_of_the_kind() {
    assert_eq!(
        cleared(
            &Shipment::default(),
            FieldBehaviorKind::Immutable,
            &["origin_carrier.name", "notes"]
        ),
        ["notes"]
    );
}

#[test]
fn clear_field_mask_keeps_a_path_that_names_no_field_and_a_star() {
    let paths = ["*", "unknown", "carrier.unknown", "notes.length"];
    assert_eq!(
        cleared(&Shipment::default(), FieldBehaviorKind::OutputOnly, &paths),
        paths
    );
}

#[test]
fn clear_field_mask_edits_the_mask_in_place_and_leaves_the_message_alone() {
    let shipment = Shipment {
        create_time: Some(7),
        ..Default::default()
    };
    let mut mask = mask(&["create_time", "notes"]);
    shipment.clear_field_mask(FieldBehaviorKind::OutputOnly, &mut mask);
    assert_eq!(mask.paths, ["notes"]);
    assert_eq!(shipment.create_time, Some(7));
}

#[test]
fn clear_field_mask_leaves_an_empty_mask_and_a_mask_of_no_kind_alone() {
    assert!(cleared(&Shipment::default(), FieldBehaviorKind::OutputOnly, &[]).is_empty());

    assert_eq!(
        cleared(
            &Shipment::default(),
            FieldBehaviorKind::Required,
            &["create_time", "carrier.name"]
        ),
        ["create_time", "carrier.name"]
    );
}

#[test]
fn clear_field_mask_reads_the_schema_not_the_values() {
    let shipment = Shipment {
        create_time: Some(7),
        carrier: Some(Carrier {
            tracking_id: "T-1".to_owned(),
        }),
        ..Default::default()
    };
    assert_eq!(
        cleared(
            &shipment,
            FieldBehaviorKind::OutputOnly,
            &["create_time", "carrier.tracking_id", "carrier", "notes"]
        ),
        ["carrier", "notes"]
    );
}
