use rutile_core::{
    Document, Edit, EditError, EditTransaction, MAX_DOCUMENT_BYTES, TransactionKind,
};

fn transaction(document: &Document, id: u64, edits: Vec<Edit>) -> EditTransaction {
    EditTransaction {
        base_revision: document.revision(),
        id,
        kind: TransactionKind::Programmatic,
        edits,
    }
}

fn boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect()
}

#[test]
fn randomized_multi_edit_transactions_match_string_oracle() {
    let mut document = Document::new("α bravo 🪶 charlie 日本 delta").unwrap();
    let mut oracle = document.snapshot().to_string();
    let mut random = 0x243f_6a88_85a3_08d3_u64;

    for id in 1..=200 {
        let points = boundaries(&oracle);
        let mut indexes = [0usize; 4];
        for index in &mut indexes {
            random = random
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            *index = (random as usize) % points.len();
        }
        indexes.sort_unstable();
        let first = points[indexes[0]]..points[indexes[1]];
        let second = points[indexes[2]]..points[indexes[3]];
        let replacements = if id % 2 == 0 {
            ("日本", "x")
        } else {
            ("🪶", "é")
        };
        let edits = vec![
            Edit {
                byte_range: first.clone(),
                replacement: replacements.0.into(),
            },
            Edit {
                byte_range: second.clone(),
                replacement: replacements.1.into(),
            },
        ];

        oracle.replace_range(second, replacements.1);
        oracle.replace_range(first, replacements.0);
        document.apply(transaction(&document, id, edits)).unwrap();

        assert_eq!(document.snapshot().to_string(), oracle, "transaction {id}");
    }
}

#[test]
fn exact_twenty_mib_boundary_is_accepted_and_one_more_byte_is_atomic() {
    let exact = "x".repeat(MAX_DOCUMENT_BYTES);
    let mut document = Document::new(&exact).unwrap();

    document
        .apply(transaction(
            &document,
            1,
            vec![Edit {
                byte_range: MAX_DOCUMENT_BYTES - 1..MAX_DOCUMENT_BYTES,
                replacement: "y".into(),
            }],
        ))
        .unwrap();
    let before = document.snapshot().to_string();
    let error = document
        .apply(transaction(
            &document,
            2,
            vec![Edit {
                byte_range: MAX_DOCUMENT_BYTES..MAX_DOCUMENT_BYTES,
                replacement: "z".into(),
            }],
        ))
        .unwrap_err();

    assert_eq!(error, EditError::TooLarge);
    assert_eq!(document.len_bytes(), MAX_DOCUMENT_BYTES);
    assert_eq!(document.snapshot().to_string(), before);
    document.undo().unwrap();
    assert_eq!(document.snapshot().to_string(), exact);
}

#[test]
fn five_mib_edit_sequence_matches_string_oracle() {
    const FIVE_MIB: usize = 5 * 1024 * 1024;
    let mut oracle = "a".repeat(FIVE_MIB);
    let mut document = Document::new(&oracle).unwrap();

    for id in 1..=64 {
        let start = (id as usize * 65_537) % (FIVE_MIB - 4);
        let replacement = if id % 2 == 0 { "日" } else { "🪶" };
        oracle.replace_range(start..start + 1, replacement);
        document
            .apply(transaction(
                &document,
                id,
                vec![Edit {
                    byte_range: start..start + 1,
                    replacement: replacement.into(),
                }],
            ))
            .unwrap();
    }

    assert_eq!(document.snapshot().to_string(), oracle);
}
