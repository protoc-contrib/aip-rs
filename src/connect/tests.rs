use super::*;
use crate::FieldBehaviorError;

/// A resource with one `REQUIRED` field, `title` (3), an `OUTPUT_ONLY`
/// `create_time` (4) and an `INPUT_ONLY` `secret`.
#[derive(Default)]
struct Book {
    name: String,
    title: String,
    create_time: String,
    secret: String,
}

impl FieldBehavior for Book {
    fn clear_fields(&mut self, kind: FieldBehaviorKind) {
        if kind == FieldBehaviorKind::OutputOnly {
            self.create_time.clear();
        }
        if kind == FieldBehaviorKind::InputOnly {
            self.secret.clear();
        }
    }

    fn validate_fields(
        &self,
        kind: FieldBehaviorKind,
        scope: FieldScope<'_>,
    ) -> Result<(), FieldBehaviorError> {
        let mut paths = Vec::new();
        if kind == FieldBehaviorKind::Required && scope.contains("title") && self.title.is_empty() {
            paths.push(FieldPath::field("title", 3, FieldType::String));
        }
        if kind == FieldBehaviorKind::OutputOnly
            && scope.contains("create_time")
            && self.create_time.is_empty()
        {
            paths.push(FieldPath::field("create_time", 4, FieldType::String));
        }
        FieldBehaviorError::check(kind, paths)
    }
}

impl Resource for Book {
    fn implied_field_mask(&self) -> FieldMask {
        let mut paths = Vec::new();
        if !self.title.is_empty() {
            paths.push("title".to_owned());
        }
        FieldMask {
            paths,
            ..Default::default()
        }
    }
}

fn book_path() -> FieldPath {
    FieldPath::field("book", 2, FieldType::Message)
}

#[derive(Default)]
struct CreateBook {
    book_id: String,
    book: Option<Book>,
}

impl FieldBehavior for CreateBook {
    fn clear_fields(&mut self, kind: FieldBehaviorKind) {
        if let Some(book) = &mut self.book {
            book.clear_fields(kind);
        }
    }
}

impl ResourceWrites for CreateBook {
    fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
        if let Some(book) = &mut self.book {
            f(ResourceWrite {
                path: book_path(),
                resource: book,
                method: WriteMethod::Create {
                    id: Some(&mut self.book_id),
                },
            });
        }
    }
}

impl Validate for CreateBook {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Default)]
struct UpdateBook {
    book: Option<Book>,
    update_mask: Option<FieldMask>,
}

impl FieldBehavior for UpdateBook {}

impl ResourceWrites for UpdateBook {
    fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
        if let Some(book) = &mut self.book {
            f(ResourceWrite {
                path: book_path(),
                resource: book,
                method: WriteMethod::Update {
                    mask: self.update_mask.get_or_insert_default(),
                },
            });
        }
    }
}

impl Validate for UpdateBook {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Default)]
struct BatchCreateBooks {
    requests: Vec<CreateBook>,
}

impl FieldBehavior for BatchCreateBooks {}

impl ResourceWrites for BatchCreateBooks {
    fn for_each_write(&mut self, f: &mut dyn FnMut(ResourceWrite<'_>)) {
        for (index, request) in self.requests.iter_mut().enumerate() {
            request.for_each_write(&mut |write| {
                f(write.prefixed(&FieldPath::index("requests", 1, FieldType::Message, index)));
            });
        }
    }
}

impl Validate for BatchCreateBooks {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

fn mask(paths: &[&str]) -> Option<FieldMask> {
    Some(FieldMask {
        paths: paths.iter().map(|&path| path.to_owned()).collect(),
        ..Default::default()
    })
}

#[test]
fn a_create_mints_an_omitted_id_and_leaves_the_name_alone() {
    let mut request = CreateBook {
        book: Some(Book::default()),
        ..Default::default()
    };
    imply(&mut request, &mut || "minted".to_owned());
    assert_eq!(request.book_id, "minted");
    assert_eq!(request.book.expect("the book stays").name, "");
}

#[test]
fn a_create_keeps_the_id_the_client_sent() {
    let mut request = CreateBook {
        book_id: "sent".to_owned(),
        book: Some(Book::default()),
    };
    imply(&mut request, &mut || unreachable!("an id was sent"));
    assert_eq!(request.book_id, "sent");
}

#[test]
fn an_update_implies_an_omitted_mask_and_keeps_a_sent_one() {
    let mut request = UpdateBook {
        book: Some(Book {
            title: "Dune".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    imply(&mut request, &mut String::new);
    assert_eq!(
        request.update_mask.map(|mask| mask.paths),
        Some(vec!["title".to_owned()])
    );

    let mut sent = UpdateBook {
        book: Some(Book::default()),
        update_mask: mask(&["other"]),
    };
    imply(&mut sent, &mut String::new);
    assert_eq!(
        sent.update_mask.map(|mask| mask.paths),
        Some(vec!["other".to_owned()])
    );
}

#[test]
fn an_update_ignores_the_output_only_fields_an_explicit_mask_names() {
    let mut request = UpdateBook {
        book: Some(Book {
            title: "Dune".to_owned(),
            ..Default::default()
        }),
        update_mask: mask(&["create_time", "title", "other"]),
    };
    imply(&mut request, &mut String::new);
    assert_eq!(
        request.update_mask.map(|mask| mask.paths),
        Some(vec!["title".to_owned(), "other".to_owned()])
    );
}

#[test]
fn an_update_leaves_a_mask_of_only_output_only_fields_empty_rather_than_implied() {
    let mut request = UpdateBook {
        book: Some(Book {
            title: "Dune".to_owned(),
            ..Default::default()
        }),
        update_mask: mask(&["create_time"]),
    };
    imply(&mut request, &mut String::new);
    assert_eq!(request.update_mask.map(|mask| mask.paths), Some(Vec::new()));
}

#[test]
fn a_create_requires_every_required_field() {
    let mut request = CreateBook {
        book: Some(Book::default()),
        ..Default::default()
    };
    assert_eq!(
        required_paths(&mut request),
        [book_path().join(FieldPath::field("title", 3, FieldType::String))]
    );
}

#[test]
fn an_update_requires_only_the_fields_its_mask_names() {
    let mut other = UpdateBook {
        book: Some(Book::default()),
        update_mask: mask(&["other"]),
    };
    assert!(required_paths(&mut other).is_empty());

    let mut named = UpdateBook {
        book: Some(Book::default()),
        update_mask: mask(&["title"]),
    };
    assert_eq!(required_paths(&mut named).len(), 1);
}

#[test]
fn a_batch_reports_each_entry_under_its_index() {
    let mut request = BatchCreateBooks {
        requests: vec![
            CreateBook {
                book: Some(Book {
                    title: "Dune".to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            CreateBook {
                book: Some(Book::default()),
                ..Default::default()
            },
        ],
    };
    let paths = required_paths(&mut request);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].to_string(), "requests[1].book.title");
}

#[test]
fn a_bad_request_names_each_violation_s_field_and_message() {
    let title = FieldPath::index("requests", 1, FieldType::Message, 2)
        .join(book_path())
        .join(FieldPath::field("title", 3, FieldType::String));
    let request = bad_request(&[required_violation(&title)]);
    let violations: Vec<_> = request
        .field_violations
        .iter()
        .map(|violation| (violation.field.as_str(), violation.description.as_str()))
        .collect();
    assert_eq!(
        violations,
        [("requests[2].book.title", "value is required")]
    );
}

#[test]
fn a_bad_request_stops_short_of_the_detail_budget() {
    let title = FieldPath::field("title", 3, FieldType::String);
    let violations = vec![required_violation(&title); 1000];
    let request = bad_request(&violations);
    assert!(!request.field_violations.is_empty());
    assert!(request.field_violations.len() < violations.len());
    assert!(request.encoded_len() as usize <= MAX_DETAIL_BYTES);
}

#[test]
fn a_required_violation_reads_like_protovalidate_s_own() {
    let path = FieldPath::index("requests", 1, FieldType::Message, 2)
        .join(book_path())
        .join(FieldPath::field("title", 3, FieldType::String));
    let violation = required_violation(&path);
    assert_eq!(violation.rule_id, "required");
    assert_eq!(violation.message, "value is required");
    assert_eq!(violation.field.to_string(), "requests[2].book.title");
    let elements: Vec<_> = violation
        .field
        .elements
        .iter()
        .map(|element| {
            (
                element.field_name.as_deref().unwrap_or_default().to_owned(),
                element.field_number,
                element.field_type,
            )
        })
        .collect();
    assert_eq!(
        elements,
        [
            ("requests".to_owned(), Some(1), Some(ViolationType::Message)),
            ("book".to_owned(), Some(2), Some(ViolationType::Message)),
            ("title".to_owned(), Some(3), Some(ViolationType::String)),
        ]
    );
    assert!(matches!(
        violation.field.elements[0].subscript,
        Some(Subscript::Index(2))
    ));
}

#[test]
fn a_field_type_converts_to_protovalidate_s() {
    assert_eq!(
        ViolationType::from(FieldType::Sint64),
        ViolationType::Sint64
    );
    assert_eq!(ViolationType::from(FieldType::Group), ViolationType::Group);
}

#[test]
fn every_resource_is_a_dyn_resource() {
    let book = Book {
        title: "Dune".to_owned(),
        ..Default::default()
    };
    let resource: &dyn DynResource = &book;
    assert_eq!(resource.implied_field_mask().paths, ["title"]);
}

mod interceptor;
