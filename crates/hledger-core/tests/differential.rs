//! Differential tests: compare this engine's numbers against the real
//! hledger CLI on the repo fixtures. These are the ground-truth tests the
//! audit called for — the unit suite passed while balances were wrong.
//!
//! Tests are skipped (with a note) when hledger is not installed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_decimal::Decimal;

fn hledger_available() -> bool {
    Command::new("hledger")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Parse hledger's `bal --flat -N -O csv` output into account → commodity → qty.
fn hledger_balances(file: &Path) -> BTreeMap<String, BTreeMap<String, Decimal>> {
    let output = Command::new("hledger")
        .args(["-f", &file.to_string_lossy(), "bal", "--flat", "-N", "-O", "csv"])
        .output()
        .expect("failed to run hledger");
    assert!(
        output.status.success(),
        "hledger bal failed on {}: {}",
        file.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    let text = String::from_utf8_lossy(&output.stdout);
    let mut result = BTreeMap::new();

    for line in text.lines().skip(1) {
        // Format: "account","balance" — split on the quoted separator.
        let line = line.trim();
        let Some(stripped) = line.strip_prefix('"').and_then(|l| l.strip_suffix('"')) else {
            continue;
        };
        let Some((account, balance)) = stripped.split_once("\",\"") else {
            continue;
        };

        let entry: &mut BTreeMap<String, Decimal> =
            result.entry(account.to_string()).or_default();

        // A multi-commodity cell separates amounts with ", " or newlines.
        for part in balance.split(['\n']).flat_map(|p| p.split(", ")) {
            let part = part.trim();
            if part.is_empty() || part == "0" {
                continue;
            }
            let amt = hledger_parser::parse_amount(part)
                .unwrap_or_else(|e| panic!("cannot parse hledger amount '{}': {}", part, e));
            *entry.entry(amt.commodity).or_insert(Decimal::ZERO) += amt.quantity;
        }
    }

    result
}

/// Our engine's flat balances for the same file (through the real parser and
/// resolver, includes resolved like the app does not — fixtures are single
/// files).
fn engine_balances(file: &Path) -> BTreeMap<String, BTreeMap<String, Decimal>> {
    let text = std::fs::read_to_string(file).expect("read fixture");
    let journal = hledger_parser::parse(&text)
        .unwrap_or_else(|e| panic!("engine failed to parse {}: {}", file.display(), e));
    let txns = hledger_core::balance::resolve_transactions(&journal)
        .unwrap_or_else(|e| panic!("engine failed to resolve {}: {}", file.display(), e));

    let mut result: BTreeMap<String, BTreeMap<String, Decimal>> = BTreeMap::new();
    for txn in &txns {
        for posting in &txn.postings {
            let entry = result.entry(posting.account.full.clone()).or_default();
            for (commodity, qty) in &posting.amount.amounts {
                *entry.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
            }
        }
    }
    // Drop zero entries like hledger's flat report does.
    for by_commodity in result.values_mut() {
        by_commodity.retain(|_, q| !q.is_zero());
    }
    result.retain(|_, m| !m.is_empty());
    result
}

fn assert_balances_match(file: &Path) {
    let ours = engine_balances(file);
    let theirs = hledger_balances(file);

    for (account, their_amounts) in &theirs {
        let our_amounts = ours.get(account).unwrap_or_else(|| {
            panic!(
                "{}: hledger reports account '{}' but the engine has no balance for it",
                file.display(),
                account
            )
        });
        for (commodity, their_qty) in their_amounts {
            let our_qty = our_amounts.get(commodity).copied().unwrap_or(Decimal::ZERO);
            assert_eq!(
                &our_qty, their_qty,
                "{}: balance mismatch for {} in {}: engine={}, hledger={}",
                file.display(),
                account,
                commodity,
                our_qty,
                their_qty
            );
        }
        // No commodities hledger doesn't have.
        for commodity in our_amounts.keys() {
            assert!(
                their_amounts.contains_key(commodity),
                "{}: engine reports {} {} that hledger does not",
                file.display(),
                account,
                commodity
            );
        }
    }

    for account in ours.keys() {
        assert!(
            theirs.contains_key(account),
            "{}: engine reports account '{}' that hledger does not",
            file.display(),
            account
        );
    }
}

fn all_fixtures() -> Vec<PathBuf> {
    ["basic.journal",
     "assertions.journal",
     "edge_cases.journal",
     "multicurrency.journal",
     "sample-with-budget.journal",
     "example.hledger"]
        .iter()
        .map(|n| fixture(n))
        .collect()
}

#[test]
fn balances_match_hledger_on_all_fixtures() {
    if !hledger_available() {
        eprintln!("hledger CLI not found — skipping differential test");
        return;
    }
    for file in all_fixtures() {
        assert_balances_match(&file);
    }
}

#[test]
fn transaction_counts_match_hledger() {
    if !hledger_available() {
        eprintln!("hledger CLI not found — skipping differential test");
        return;
    }
    for file in all_fixtures() {
        let output = Command::new("hledger")
            .args(["-f", &file.to_string_lossy(), "print"])
            .output()
            .expect("run hledger print");
        let their_count = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
            .count();

        let text = std::fs::read_to_string(&file).unwrap();
        let journal = hledger_parser::parse(&text).unwrap();
        let our_count = journal
            .items
            .iter()
            .filter(|i| matches!(i, hledger_parser::ast::JournalItem::Transaction(_)))
            .count();

        assert_eq!(
            our_count,
            their_count,
            "{}: transaction count differs (engine={}, hledger={})",
            file.display(),
            our_count,
            their_count
        );
    }
}

#[test]
fn fixtures_parse_without_warnings() {
    for file in all_fixtures() {
        let text = std::fs::read_to_string(&file).unwrap();
        let journal = hledger_parser::parse(&text)
            .unwrap_or_else(|e| panic!("{} failed to parse: {}", file.display(), e));
        assert!(
            journal.warnings.is_empty(),
            "{}: unexpected parse warnings: {:?}",
            file.display(),
            journal.warnings
        );
    }
}

/// The audit's headline defect: net worth on the sample investment journal
/// showed ~$4,094 where hledger reports ~$79,450 (holdings were never valued
/// and different commodities were raw-summed). Pin the corrected number to
/// hledger's own valued balance.
#[test]
fn net_worth_matches_hledger_valued_balance() {
    if !hledger_available() {
        eprintln!("hledger CLI not found — skipping differential test");
        return;
    }

    let file = fixture("example.hledger");
    let text = std::fs::read_to_string(&file).unwrap();
    let journal = hledger_parser::parse(&text).unwrap();
    let txns = hledger_core::balance::resolve_transactions(&journal).unwrap();
    let classifier = hledger_core::classify::AccountClassifier::from_journal(&journal);
    let price_db = hledger_core::price_db::PriceDb::from_journal(&journal);

    let series = hledger_core::reports::net_worth_series(
        &txns, &classifier, &price_db, "USD", None, None,
    );
    let ours: f64 = series.last().unwrap().value.parse().unwrap();

    // hledger bal assets liabilities -V --flat: USD component of the total.
    let output = Command::new("hledger")
        .args(["-f", &file.to_string_lossy(), "bal", "assets", "liabilities", "-V", "--flat"])
        .output()
        .expect("run hledger bal -V");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let theirs: f64 = stdout
        .lines()
        .rev()
        .find_map(|l| {
            let l = l.trim();
            l.strip_suffix(" USD").and_then(|n| n.trim().parse::<f64>().ok())
        })
        .expect("USD total line in hledger output");

    assert!(
        (ours - theirs).abs() < 0.01,
        "net worth: engine={}, hledger={}",
        ours,
        theirs
    );
}

/// Writing a transaction back must produce text hledger itself accepts.
#[test]
fn rewritten_transactions_stay_hledger_valid() {
    if !hledger_available() {
        eprintln!("hledger CLI not found — skipping differential test");
        return;
    }

    for file in all_fixtures() {
        let text = std::fs::read_to_string(&file).unwrap();
        let journal = hledger_parser::parse(&text).unwrap();
        let config = hledger_parser::writer::infer_config(&text);

        // Rewrite every transaction in place via the span patcher.
        let mut patches = Vec::new();
        for item in &journal.items {
            if let hledger_parser::ast::JournalItem::Transaction(t) = item {
                patches.push((t.span.clone(), hledger_parser::writer::write_transaction(t, &config)));
            }
        }
        let rewritten = hledger_parser::writer::patch_journal(&text, &patches)
            .unwrap_or_else(|e| panic!("{}: patch failed: {}", file.display(), e));

        // hledger must still accept the file and report identical balances.
        let tmp = std::env::temp_dir().join(format!(
            "pockethledger-difftest-{}-{}",
            std::process::id(),
            file.file_name().unwrap().to_string_lossy()
        ));
        std::fs::write(&tmp, &rewritten).unwrap();

        let check = Command::new("hledger")
            .args(["-f", &tmp.to_string_lossy(), "check"])
            .output()
            .expect("run hledger check");
        assert!(
            check.status.success(),
            "{}: hledger rejects the rewritten journal:\n{}",
            file.display(),
            String::from_utf8_lossy(&check.stderr)
        );

        let before = hledger_balances(&file);
        let after = hledger_balances(&tmp);
        assert_eq!(
            before,
            after,
            "{}: balances changed after rewrite",
            file.display()
        );

        let _ = std::fs::remove_file(&tmp);
    }
}
