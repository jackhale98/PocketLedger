use hledger_parser::ast::JournalItem;
use hledger_parser::parse;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    ))
    .unwrap()
}

#[test]
fn parse_basic_journal() {
    let input = fixture("basic.journal");
    let journal = parse(&input).unwrap();

    let transactions: Vec<_> = journal
        .items
        .iter()
        .filter(|i| matches!(i, JournalItem::Transaction(_)))
        .collect();

    assert_eq!(transactions.len(), 8, "basic.journal should have 8 transactions");
}

#[test]
fn parse_multicurrency_journal() {
    let input = fixture("multicurrency.journal");
    let journal = parse(&input).unwrap();

    let transactions: Vec<_> = journal
        .items
        .iter()
        .filter(|i| matches!(i, JournalItem::Transaction(_)))
        .collect();

    assert_eq!(transactions.len(), 7, "multicurrency.journal should have 7 transactions");

    // Verify price directives
    let prices: Vec<_> = journal
        .items
        .iter()
        .filter(|i| matches!(i, JournalItem::PriceDirective(_)))
        .collect();

    assert_eq!(prices.len(), 6, "multicurrency.journal should have 6 price directives");
}

#[test]
fn parse_assertions_journal() {
    let input = fixture("assertions.journal");
    let journal = parse(&input).unwrap();

    let transactions: Vec<_> = journal
        .items
        .iter()
        .filter(|i| matches!(i, JournalItem::Transaction(_)))
        .collect();

    assert_eq!(transactions.len(), 4, "assertions.journal should have 4 transactions");

    // Verify balance assertions are parsed
    if let JournalItem::Transaction(txn) = &transactions[0] {
        assert!(
            txn.postings[0].balance_assertion.is_some(),
            "First posting should have a balance assertion"
        );
    }
}

#[test]
fn parse_edge_cases_journal() {
    let input = fixture("edge_cases.journal");
    let journal = parse(&input).unwrap();

    let transactions: Vec<_> = journal
        .items
        .iter()
        .filter(|i| matches!(i, JournalItem::Transaction(_)))
        .collect();

    // Should parse all edge case transactions without error
    assert!(
        transactions.len() >= 8,
        "edge_cases.journal should have at least 8 transactions, got {}",
        transactions.len()
    );
}
