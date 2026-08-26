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

/// Multi-period balance report vs `hledger bal -M --flat`: every cell of
/// every fixture must agree.
#[test]
fn monthly_periodic_report_matches_hledger() {
    if !hledger_available() {
        eprintln!("hledger CLI not found — skipping differential test");
        return;
    }

    for file in all_fixtures() {
        // hledger side: "account","2024-01","2024-02",... rows.
        let output = Command::new("hledger")
            .args(["-f", &file.to_string_lossy(), "bal", "-M", "--flat", "-N", "-O", "csv"])
            .output()
            .expect("run hledger bal -M");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let header = lines.next().unwrap_or_default();
        let periods: Vec<String> = header
            .trim_start_matches('"')
            .trim_end_matches('"')
            .split("\",\"")
            .skip(1)
            .map(|s| s.to_string())
            .collect();

        let mut theirs: BTreeMap<String, Vec<BTreeMap<String, Decimal>>> = BTreeMap::new();
        for line in lines {
            let Some(stripped) = line.trim().strip_prefix('"').and_then(|l| l.strip_suffix('"'))
            else {
                continue;
            };
            let cells: Vec<&str> = stripped.split("\",\"").collect();
            if cells.len() != periods.len() + 1 {
                continue;
            }
            let mut row = Vec::new();
            for cell in &cells[1..] {
                let mut amounts = BTreeMap::new();
                for part in cell.split(['\n']).flat_map(|p| p.split(", ")) {
                    let part = part.trim();
                    if part.is_empty() || part == "0" {
                        continue;
                    }
                    let amt = hledger_parser::parse_amount(part)
                        .unwrap_or_else(|e| panic!("bad amount '{}': {}", part, e));
                    *amounts.entry(amt.commodity).or_insert(Decimal::ZERO) += amt.quantity;
                }
                row.push(amounts);
            }
            theirs.insert(cells[0].to_string(), row);
        }

        // Engine side.
        let text = std::fs::read_to_string(&file).unwrap();
        let journal = hledger_parser::parse(&text).unwrap();
        let txns = hledger_core::balance::resolve_transactions(&journal).unwrap();
        let report = hledger_core::periodic_report::periodic_balance_report(
            &txns,
            hledger_core::periodic_report::ReportInterval::Monthly,
            hledger_core::periodic_report::AccumulationMode::Periodic,
            None,
            None,
            None,
            None,
            // Unvalued: this compares against `hledger bal -M` without -V.
            "",
            &hledger_core::price_db::PriceDb::default(),
        );

        assert_eq!(
            report.periods, periods,
            "{}: period columns differ",
            file.display()
        );

        for (account, their_row) in &theirs {
            let our_row = report
                .rows
                .iter()
                .find(|r| &r.account == account)
                .unwrap_or_else(|| {
                    panic!("{}: engine missing account {}", file.display(), account)
                });
            for (i, their_cell) in their_row.iter().enumerate() {
                let mut ours: BTreeMap<String, Decimal> = BTreeMap::new();
                for e in &our_row.amounts[i] {
                    ours.insert(e.commodity.clone(), e.quantity.parse().unwrap());
                }
                assert_eq!(
                    &ours, their_cell,
                    "{}: {} period {} differs",
                    file.display(),
                    account,
                    report.periods[i]
                );
            }
        }
    }
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

/// Forecast dates must match `hledger print --forecast=WINDOW` exactly.
///
/// This pins the anchoring rule that is easy to get wrong: a periodic rule
/// with no `from` date inherits the forecast window's START DAY, so a window
/// opening mid-month recurs mid-month rather than snapping to the 1st.
#[test]
fn forecast_dates_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    let journal_text = "\
~ monthly  Rent
    expenses:rent  $1200.00
    assets:checking

~ every 2 weeks from 2024-01-05  Paycheck
    assets:checking  $2000.00
    income:salary

~ every 15th day of month  Card payment
    liabilities:card  $300.00
    assets:checking

~ every 31st day of month  Month end
    expenses:fees  $5.00
    assets:checking

2024-01-05 Seed
    assets:checking  $5000.00
    equity:opening
";

    let dir = std::env::temp_dir().join(format!("hledger-forecast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("forecast.journal");
    std::fs::write(&path, journal_text).unwrap();

    // Windows chosen to exercise both a period-aligned and a mid-period start.
    for (start, end) in [
        ("2024-01-06", "2024-05-01"),
        ("2024-03-15", "2024-07-01"),
        ("2024-02-01", "2024-03-01"),
        // Spans February, pinning the short-month clamping rule.
        ("2024-01-01", "2024-06-01"),
    ] {
        let output = Command::new("hledger")
            .args([
                "-f",
                &path.to_string_lossy(),
                "print",
                &format!("--forecast={start}..{end}"),
                "--verbose-tags",
            ])
            .output()
            .expect("run hledger print --forecast");
        assert!(
            output.status.success(),
            "hledger failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Collect (date, description) for generated transactions only.
        let mut expected: Vec<(chrono::NaiveDate, String)> = Vec::new();
        let mut pending: Option<(chrono::NaiveDate, String)> = None;
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("20") {
                let full = format!("20{rest}");
                let (date_part, desc) = full.split_once(' ').unwrap_or((full.as_str(), ""));
                if let Ok(date) = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d") {
                    pending = Some((date, desc.trim().to_string()));
                }
            } else if line.contains("generated-transaction:") {
                if let Some(entry) = pending.take() {
                    expected.push(entry);
                }
            }
        }
        expected.sort();

        let journal = hledger_parser::parse(journal_text).unwrap();
        let window_start = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
        // hledger's window end is exclusive; ours is inclusive.
        let window_end = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .unwrap()
            .pred_opt()
            .unwrap();
        let mut ours: Vec<(chrono::NaiveDate, String)> =
            hledger_core::forecast::forecast_transactions(&journal, window_start, window_end)
                .into_iter()
                .map(|t| (t.date, t.description))
                .collect();
        ours.sort();

        assert_eq!(
            ours, expected,
            "forecast mismatch for window {start}..{end}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A rule written by the app must be valid hledger. `check --forecast`
/// validates periodic rules (plain `check` skips them), so it catches the
/// mistakes the writer could plausibly make: a single space before the
/// description, or an elided amount that leaves the rule unbalanced.
#[test]
fn written_periodic_rules_stay_hledger_valid() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    let config = hledger_parser::writer::WriterConfig::default();
    let cases: Vec<(&str, &str, Vec<(String, Option<Decimal>, String)>)> = vec![
        (
            "monthly from 2026-01",
            "Rent",
            vec![
                ("expenses:rent".into(), Some(Decimal::new(120000, 2)), "$".into()),
                ("assets:checking".into(), None, String::new()),
            ],
        ),
        (
            "every 15th day of month from 2026-01",
            "Card payment  with awkward  spacing",
            vec![
                ("liabilities:card".into(), Some(Decimal::new(30000, 2)), "$".into()),
                ("assets:checking".into(), None, String::new()),
            ],
        ),
        (
            "every 2 weeks from 2026-01-05",
            "",
            vec![
                ("assets:checking".into(), Some(Decimal::new(200000, 2)), "$".into()),
                ("income:salary".into(), Some(Decimal::new(-200000, 2)), "$".into()),
            ],
        ),
    ];

    let dir = std::env::temp_dir().join(format!("hledger-rulewrite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (i, (period, description, postings)) in cases.iter().enumerate() {
        let rule = hledger_parser::writer::write_periodic_transaction_full(
            period,
            description,
            postings,
            &config,
        );
        let text = format!(
            "2026-01-01 Seed\n    assets:checking  $5000.00\n    equity:opening\n\n{rule}"
        );
        let path = dir.join(format!("rule-{i}.journal"));
        std::fs::write(&path, &text).unwrap();

        let output = Command::new("hledger")
            .args(["-f", &path.to_string_lossy(), "check", "--forecast"])
            .output()
            .expect("run hledger check --forecast");
        assert!(
            output.status.success(),
            "hledger rejected our rule:\n{text}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        // And it must actually generate something, not silently do nothing.
        let printed = Command::new("hledger")
            .args([
                "-f",
                &path.to_string_lossy(),
                "print",
                "--forecast=2026-01-01..2026-06-01",
            ])
            .output()
            .expect("run hledger print --forecast");
        let stdout = String::from_utf8_lossy(&printed.stdout);
        assert!(
            stdout.matches("\n\n").count() > 1,
            "rule generated no transactions:\n{text}\n{stdout}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The Forecast tab's projection window must equal `hledger --forecast -b
/// TODAY`, for both an up-to-date journal and a stale one.
///
/// hledger uses `forecastStart = max(journalEnd + 1, reportStart)`. The stale
/// case is the one that matters in practice: a journal last touched years ago
/// must project from today, not replay every month since.
#[test]
fn projection_matches_hledger_from_today() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let horizon = chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();

    let cases = [
        // Stale: last transaction years before today.
        "~ monthly from 2024-01-01  Rent\n    expenses:rent  $1800.00\n    assets:checking\n\n\
         ~ monthly from 2024-01-01  Dining out\n    expenses:dining  $400.00\n    assets:checking\n\n\
         2023-12-21 Groceries\n    expenses:food  $120.00\n    assets:checking\n",
        // Up to date: last transaction is today.
        "~ monthly  Rent\n    expenses:rent  $100.00\n    assets:checking\n\n\
         2026-08-21 Today\n    assets:checking  $500.00\n    equity:opening\n",
        // Running into the future: forecast must start after the last entry.
        "~ monthly  Rent\n    expenses:rent  $100.00\n    assets:checking\n\n\
         2027-01-01 Future\n    assets:checking  $500.00\n    equity:opening\n",
    ];

    let dir = std::env::temp_dir().join(format!("hledger-projwin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (i, text) in cases.iter().enumerate() {
        let path = dir.join(format!("proj-{i}.journal"));
        std::fs::write(&path, text).unwrap();

        let output = Command::new("hledger")
            .args([
                "-f",
                &path.to_string_lossy(),
                "print",
                "--forecast",
                "--verbose-tags",
                "-b",
                &today.to_string(),
                "-e",
                &horizon.succ_opt().unwrap().to_string(),
                "--today",
                &today.to_string(),
            ])
            .output()
            .expect("run hledger print --forecast -b today");
        assert!(
            output.status.success(),
            "hledger failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut expected: Vec<chrono::NaiveDate> = Vec::new();
        let mut pending: Option<chrono::NaiveDate> = None;
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("20") {
                let full = format!("20{rest}");
                let date_part = full.split_once(' ').map(|(d, _)| d).unwrap_or(&full);
                pending = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok();
            } else if line.contains("generated-transaction:") {
                if let Some(d) = pending.take() {
                    expected.push(d);
                }
            }
        }
        expected.sort();

        let journal = hledger_parser::parse(text).unwrap();
        let real = hledger_core::balance::resolve_transactions(&journal).unwrap();
        let mut ours: Vec<chrono::NaiveDate> =
            match hledger_core::forecast::projection_window(&real, today, Some(horizon)) {
                Some((start, end)) => {
                    hledger_core::forecast::forecast_transactions(&journal, start, end)
                        .into_iter()
                        .map(|t| t.date)
                        .collect()
                }
                None => vec![],
            };
        ours.sort();

        assert_eq!(ours, expected, "projection window mismatch for case {i}:\n{text}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Budget rows must match `hledger bal --budget` account for account.
///
/// This engine is a from-scratch reimplementation of hledger's budget
/// semantics, and the nesting rules are subtle: a parent row rolls up its
/// descendants on both the goal and the actual side. Pinning it against the
/// real thing is the only way to know the two haven't drifted.
#[test]
fn budget_rows_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    let cases: [(&str, &str, &str, &str); 3] = [
        (
            "nested",
            // A budget nested inside another: the case where every row can be
            // right and the total still wrong.
            concat!(
                "~ monthly from 2024-01-01  Fun\n",
                "    expenses:fun-money             $100.00\n",
                "    assets\n\n",
                "~ monthly from 2024-01-01  Rides\n",
                "    expenses:fun-money:ride-share   $50.00\n",
                "    assets\n\n",
                "2024-01-10 Concert\n    expenses:fun-money  $80.00\n    assets:checking\n\n",
                "2024-01-12 Uber\n    expenses:fun-money:ride-share  $30.00\n    assets:checking\n",
            ),
            "2024-01-01",
            "2024-02-01",
        ),
        (
            "multi-period",
            concat!(
                "~ monthly from 2024-01-01  Food\n",
                "    expenses:food  $200.00\n",
                "    assets\n\n",
                "2024-01-10 Shop\n    expenses:food  $150.00\n    assets:checking\n\n",
                "2024-02-10 Shop\n    expenses:food  $260.00\n    assets:checking\n\n",
                "2024-03-10 Shop\n    expenses:food  $190.00\n    assets:checking\n",
            ),
            "2024-01-01",
            "2024-04-01",
        ),
        (
            "bounded rule",
            concat!(
                "~ monthly from 2024-02-01 to 2024-04-01  Gas\n",
                "    expenses:gas  $85.00\n",
                "    assets\n\n",
                "2024-01-10 Early\n    expenses:gas  $70.00\n    assets:checking\n\n",
                "2024-02-10 In\n    expenses:gas  $90.00\n    assets:checking\n\n",
                "2024-03-10 In\n    expenses:gas  $80.00\n    assets:checking\n",
            ),
            "2024-02-01",
            "2024-04-01",
        ),
    ];

    let dir = std::env::temp_dir().join(format!("hledger-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (name, text, begin, end) in cases {
        let path = dir.join(format!("{}.journal", name.replace(' ', "-")));
        std::fs::write(&path, text).unwrap();

        let output = Command::new("hledger")
            .args([
                "-f",
                &path.to_string_lossy(),
                "bal",
                "--budget",
                "-b",
                begin,
                "-e",
                end,
                "-O",
                "csv",
            ])
            .output()
            .expect("run hledger bal --budget");
        assert!(
            output.status.success(),
            "{name}: hledger failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);

        // account -> (actual, goal), both as plain decimals.
        let mut expected: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
        for line in stdout.lines().skip(1) {
            let cols: Vec<String> = parse_csv_row(line);
            if cols.len() < 3 || cols[0] == "Total:" {
                continue;
            }
            let num = |s: &str| -> Decimal {
                let cleaned: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                cleaned.parse().unwrap_or(Decimal::ZERO)
            };
            expected.insert(cols[0].clone(), (num(&cols[1]), num(&cols[2])));
        }

        let journal = hledger_parser::parse(text).unwrap();
        let budgets = hledger_core::budget::extract_budgets(&journal);
        let txns = hledger_core::balance::resolve_transactions(&journal).unwrap();
        let report = hledger_core::budget::budget_comparison(
            &txns,
            &budgets,
            &hledger_core::price_db::PriceDb::default(),
            "$",
            Some(chrono::NaiveDate::parse_from_str(begin, "%Y-%m-%d").unwrap()),
            // hledger's -e is exclusive.
            Some(
                chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
                    .unwrap()
                    .pred_opt()
                    .unwrap(),
            ),
        );

        for row in &report.rows {
            let (their_actual, their_goal) = expected.get(&row.account).unwrap_or_else(|| {
                panic!("{name}: hledger has no row for {}; it has {:?}", row.account, expected.keys().collect::<Vec<_>>())
            });
            let ours_actual: Decimal = row.actual.parse().unwrap();
            let ours_goal: Decimal = row.budget.parse().unwrap();
            assert_eq!(
                ours_actual, *their_actual,
                "{name}: actual differs for {}",
                row.account
            );
            assert_eq!(
                ours_goal, *their_goal,
                "{name}: goal differs for {}",
                row.account
            );
        }
        assert!(!report.rows.is_empty(), "{name}: produced no rows");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Minimal CSV row splitter for hledger's quoted output.
fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

type CommodityMap = BTreeMap<String, Decimal>;

/// Split a balance cell into its commodities. A cell can hold several
/// ("$-100.00, -50 EUR"), so stripping non-digits fuses them into nonsense.
fn parse_amount_cell(cell: &str) -> CommodityMap {
    let mut out = CommodityMap::new();
    for part in cell.split('\n').flat_map(|p| p.split(", ")) {
        let part = part.trim();
        if part.is_empty() || part == "0" {
            continue;
        }
        if let Ok(amt) = hledger_parser::parse_amount(part) {
            *out.entry(amt.commodity).or_insert(Decimal::ZERO) += amt.quantity;
        }
    }
    out.retain(|_, q| !q.is_zero());
    out
}

/// Parse an `is`/`bs` CSV into section label -> account -> amounts. Section
/// headers are rows with an empty value; `Total:`/`Net:` are handled by the
/// caller.
fn parse_statement_csv(stdout: &str) -> BTreeMap<String, BTreeMap<String, CommodityMap>> {
    let mut sections: BTreeMap<String, BTreeMap<String, CommodityMap>> = BTreeMap::new();
    let mut current = String::new();

    for line in stdout.lines().skip(1) {
        let cols = parse_csv_row(line);
        if cols.len() < 2 {
            continue;
        }
        let (label, value) = (cols[0].trim(), cols[1].trim());
        if label == "Account" || label == "Total:" || label == "Net:" {
            continue;
        }
        if value.is_empty() {
            current = label.to_string();
            continue;
        }
        sections
            .entry(current.clone())
            .or_default()
            .insert(label.to_string(), parse_amount_cell(value));
    }
    sections
}

/// Roll flat rows up into inclusive ones.
///
/// hledger's CSV gives each account only its own postings; our statement rows
/// are inclusive, because collapsing a parent has to show the whole subtree.
/// Rolling hledger's numbers up the same way compares the arithmetic rather
/// than a presentation choice.
fn rolled_up(flat: &BTreeMap<String, CommodityMap>) -> BTreeMap<String, CommodityMap> {
    flat.keys()
        .map(|account| {
            let prefix = format!("{account}:");
            let mut sum = CommodityMap::new();
            for (other, amounts) in flat {
                if other == account || other.starts_with(&prefix) {
                    for (commodity, q) in amounts {
                        *sum.entry(commodity.clone()).or_insert(Decimal::ZERO) += q;
                    }
                }
            }
            sum.retain(|_, q| !q.is_zero());
            (account.clone(), sum)
        })
        .collect()
}

fn row_amounts(rows: &[hledger_core::reports::BalanceRow], account: &str) -> CommodityMap {
    rows.iter()
        .find(|r| r.account == account)
        .map(|r| {
            r.amounts
                .iter()
                .filter_map(|a| {
                    a.quantity
                        .parse::<Decimal>()
                        .ok()
                        .filter(|q| !q.is_zero())
                        .map(|q| (a.commodity.clone(), q))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compare one statement section against hledger's rows for it.
fn assert_section_matches(
    file: &Path,
    label: &str,
    expected_flat: &BTreeMap<String, CommodityMap>,
    ours: &[hledger_core::reports::BalanceRow],
) {
    for (account, amounts) in rolled_up(expected_flat) {
        assert_eq!(
            row_amounts(ours, &account),
            amounts,
            "{}: {label} row {account} differs",
            file.display()
        );
    }
}

fn statement_inputs(
    file: &Path,
) -> Option<(
    Vec<hledger_core::balance::ResolvedTransaction>,
    hledger_core::classify::AccountClassifier,
)> {
    let text = std::fs::read_to_string(file).ok()?;
    let journal = hledger_parser::parse(&text).ok()?;
    let txns = hledger_core::balance::resolve_transactions(&journal).ok()?;
    let classifier = hledger_core::classify::AccountClassifier::from_journal(&journal);
    Some((txns, classifier))
}

fn hledger_statement(file: &Path, command: &str) -> Option<BTreeMap<String, BTreeMap<String, CommodityMap>>> {
    let output = Command::new("hledger")
        .args(["-f", &file.to_string_lossy(), command, "-O", "csv"])
        .output()
        .expect("run hledger statement");
    if !output.status.success() {
        return None;
    }
    let sections = parse_statement_csv(&String::from_utf8_lossy(&output.stdout));
    (!sections.is_empty()).then_some(sections)
}

/// Income statement rows must agree with `hledger is`, including hledger's
/// convention of negating revenue for display.
#[test]
fn income_statement_matches_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }
    for file in all_fixtures() {
        let Some(sections) = hledger_statement(&file, "is") else { continue };
        let Some((txns, classifier)) = statement_inputs(&file) else { continue };
        let is = hledger_core::reports::income_statement(
            &txns,
            &classifier,
            &hledger_core::price_db::PriceDb::default(),
            "",
            None,
            None,
        );
        for (idx, label) in ["Revenues", "Expenses"].iter().enumerate() {
            if let Some(expected) = sections.get(*label) {
                assert_section_matches(&file, label, expected, &is.sections[idx].rows);
            }
        }
    }
}

/// Balance sheet rows must agree with `hledger bs`.
#[test]
fn balance_sheet_matches_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }
    for file in all_fixtures() {
        let Some(sections) = hledger_statement(&file, "bs") else { continue };
        let Some((txns, classifier)) = statement_inputs(&file) else { continue };
        let bs = hledger_core::reports::balance_sheet(
            &txns,
            &classifier,
            &hledger_core::price_db::PriceDb::default(),
            "",
            None,
            None,
        );
        for (idx, label) in ["Assets", "Liabilities"].iter().enumerate() {
            if let Some(expected) = sections.get(*label) {
                assert_section_matches(&file, label, expected, &bs.sections[idx].rows);
            }
        }
    }
}

/// Register rows and their running totals must match `hledger reg ACCOUNT`.
///
/// The running total is the part worth pinning: it depends on posting order
/// and on how multi-commodity balances accumulate, neither of which a
/// balance-only comparison exercises.
#[test]
fn register_running_totals_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };

        // Every account with postings, so this isn't pinned to one shape.
        let mut accounts: Vec<String> = txns
            .iter()
            .flat_map(|t| t.postings.iter().map(|p| p.account.full.clone()))
            .collect();
        accounts.sort();
        accounts.dedup();

        for account in accounts.iter().take(6) {
            let output = Command::new("hledger")
                .args(["-f", &file.to_string_lossy(), "reg", account, "-O", "csv"])
                .output()
                .expect("run hledger reg");
            if !output.status.success() {
                continue;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);

            // (date, amount, running total) per row.
            let expected: Vec<(String, CommodityMap, CommodityMap)> = stdout
                .lines()
                .skip(1)
                .filter_map(|line| {
                    let c = parse_csv_row(line);
                    (c.len() >= 7).then(|| {
                        (c[1].clone(), parse_amount_cell(&c[5]), parse_amount_cell(&c[6]))
                    })
                })
                .collect();
            if expected.is_empty() {
                continue;
            }

            let ours = hledger_core::reports::register_report(&txns, account, None, None);
            assert_eq!(
                ours.len(),
                expected.len(),
                "{}: {account} row count differs",
                file.display()
            );

            for (i, (date, amount, total)) in expected.iter().enumerate() {
                let row = &ours[i];
                assert_eq!(&row.date, date, "{}: {account} row {i} date", file.display());
                let got_amount: CommodityMap = row
                    .amount
                    .iter()
                    .filter_map(|a| {
                        a.quantity
                            .parse::<Decimal>()
                            .ok()
                            .filter(|q| !q.is_zero())
                            .map(|q| (a.commodity.clone(), q))
                    })
                    .collect();
                let got_total: CommodityMap = row
                    .running_total
                    .iter()
                    .filter_map(|a| {
                        a.quantity
                            .parse::<Decimal>()
                            .ok()
                            .filter(|q| !q.is_zero())
                            .map(|q| (a.commodity.clone(), q))
                    })
                    .collect();
                assert_eq!(&got_amount, amount, "{}: {account} row {i} amount", file.display());
                assert_eq!(&got_total, total, "{}: {account} row {i} running total", file.display());
            }
        }
    }
}

/// A depth-limited balance must match `hledger bal --depth N`, which clips
/// account names and folds descendants into the clipped parent.
#[test]
fn depth_limited_balances_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    for file in all_fixtures() {
        for depth in 1..=3usize {
            let output = Command::new("hledger")
                .args([
                    "-f",
                    &file.to_string_lossy(),
                    "bal",
                    "--flat",
                    "-N",
                    "--depth",
                    &depth.to_string(),
                    "-O",
                    "csv",
                ])
                .output()
                .expect("run hledger bal --depth");
            if !output.status.success() {
                continue;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let expected: BTreeMap<String, CommodityMap> = stdout
                .lines()
                .skip(1)
                .filter_map(|line| {
                    let c = parse_csv_row(line);
                    (c.len() >= 2 && c[0] != "Total:")
                        .then(|| (c[0].clone(), parse_amount_cell(&c[1])))
                })
                .collect();
            if expected.is_empty() {
                continue;
            }

            let Some((txns, _)) = statement_inputs(&file) else { continue };
            let query = hledger_core::query::parse_query(&format!("depth:{depth}")).unwrap();
            let report = hledger_core::periodic_report::periodic_balance_report(
                &txns,
                hledger_core::periodic_report::ReportInterval::Yearly,
                hledger_core::periodic_report::AccumulationMode::Historical,
                Some(depth),
                Some(&query),
                None,
                None,
                "",
                &hledger_core::price_db::PriceDb::default(),
            );

            // Historical mode's last column is the running balance to date,
            // which is what `bal` reports.
            for (account, amounts) in &expected {
                let row = report.rows.iter().find(|r| &r.account == account);
                let got: CommodityMap = row
                    .and_then(|r| r.total.last().map(|_| r.total.clone()))
                    .map(|total| {
                        total
                            .iter()
                            .filter_map(|a| {
                                a.quantity
                                    .parse::<Decimal>()
                                    .ok()
                                    .filter(|q| !q.is_zero())
                                    .map(|q| (a.commodity.clone(), q))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                assert_eq!(
                    &got,
                    amounts,
                    "{}: depth {depth} account {account} differs",
                    file.display()
                );
            }
        }
    }
}

/// Run `hledger bal` with extra arguments and return account -> commodity -> qty.
fn hledger_balances_with(file: &Path, extra: &[&str]) -> Option<BTreeMap<String, CommodityMap>> {
    let mut args = vec!["-f", &*file.to_string_lossy(), "bal", "--flat", "-N", "-O", "csv"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    args.extend(extra.iter().map(|s| s.to_string()));

    let output = Command::new("hledger").args(&args).output().expect("run hledger bal");
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let c = parse_csv_row(line);
                (c.len() >= 2 && c[0] != "Total:").then(|| (c[0].clone(), parse_amount_cell(&c[1])))
            })
            .filter(|(_, m)| !m.is_empty())
            .collect(),
    )
}

fn engine_balances_filtered(
    txns: &[hledger_core::balance::ResolvedTransaction],
) -> BTreeMap<String, CommodityMap> {
    let mut result: BTreeMap<String, CommodityMap> = BTreeMap::new();
    for txn in txns {
        for posting in &txn.postings {
            let entry = result.entry(posting.account.full.clone()).or_default();
            for (commodity, qty) in &posting.amount.amounts {
                *entry.entry(commodity.clone()).or_insert(Decimal::ZERO) += qty;
            }
        }
    }
    for m in result.values_mut() {
        m.retain(|_, q| !q.is_zero());
    }
    result.retain(|_, m| !m.is_empty());
    result
}

/// Our query language must select the same postings hledger's does.
///
/// This one has the widest blast radius in the suite: the query now filters
/// every report, so a divergence here is wrong everywhere at once.
#[test]
fn query_filtering_matches_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    let queries = [
        "acct:expenses",
        "acct:assets",
        "cur:$",
        "amt:>100",
        "amt:<0",
        "not:acct:expenses",
        "desc:Grocery",
        "status:*",
        "date:2024",
    ];

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };

        for q in queries {
            let Some(expected) = hledger_balances_with(&file, &[q]) else { continue };
            let Ok(query) = hledger_core::query::parse_query(q) else { continue };
            let filtered = hledger_core::query::retain_matching_postings(&txns, &query);
            let ours = engine_balances_filtered(&filtered);

            assert_eq!(
                ours,
                expected,
                "{}: query '{q}' selects different postings",
                file.display()
            );
        }
    }
}

/// Cumulative and historical accumulation must match `bal --cumulative` / `-H`.
///
/// Only the periodic mode had a check; these two are what the Table and
/// Account reports use for "balance as of", where an off-by-one period would
/// be invisible in a single-column comparison.
#[test]
fn accumulation_modes_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }
    use hledger_core::periodic_report::{AccumulationMode, ReportInterval};

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };

        for (flag, mode) in [
            ("--cumulative", AccumulationMode::Cumulative),
            ("-H", AccumulationMode::Historical),
        ] {
            let output = Command::new("hledger")
                .args([
                    "-f", &file.to_string_lossy(), "bal", "--flat", "-M", flag, "-O", "csv",
                ])
                .output()
                .expect("run hledger bal");
            if !output.status.success() {
                continue;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut lines = stdout.lines();
            let Some(header) = lines.next() else { continue };
            let periods: Vec<String> = parse_csv_row(header).into_iter().skip(1).collect();
            if periods.is_empty() {
                continue;
            }

            let expected: BTreeMap<String, Vec<CommodityMap>> = lines
                .filter_map(|line| {
                    let c = parse_csv_row(line);
                    (c.len() >= 2 && c[0] != "Total:").then(|| {
                        (c[0].clone(), c[1..].iter().map(|v| parse_amount_cell(v)).collect())
                    })
                })
                .collect();

            let report = hledger_core::periodic_report::periodic_balance_report(
                &txns,
                ReportInterval::Monthly,
                mode,
                None,
                None,
                None,
                None,
                "",
                &hledger_core::price_db::PriceDb::default(),
            );

            assert_eq!(
                report.periods, periods,
                "{}: {flag} period columns differ",
                file.display()
            );

            for (account, cells) in &expected {
                let Some(row) = report.rows.iter().find(|r| &r.account == account) else {
                    continue;
                };
                for (i, cell) in cells.iter().enumerate() {
                    let got: CommodityMap = row.amounts[i]
                        .iter()
                        .filter_map(|a| {
                            a.quantity
                                .parse::<Decimal>()
                                .ok()
                                .filter(|q| !q.is_zero())
                                .map(|q| (a.commodity.clone(), q))
                        })
                        .collect();
                    assert_eq!(
                        &got, cell,
                        "{}: {flag} {account} period {} differs",
                        file.display(),
                        periods[i]
                    );
                }
            }
        }
    }
}

/// Valuing in a target commodity must match `hledger bal -X CUR`.
///
/// Valuation now applies to every report, so a conversion or price-lookup
/// difference is wrong everywhere rather than in one chart.
#[test]
fn valued_balances_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };
        let text = std::fs::read_to_string(&file).unwrap();
        let journal = hledger_parser::parse(&text).unwrap();
        let price_db = hledger_core::price_db::PriceDb::from_journal(&journal);

        // Commodities actually priced in this fixture; valuing into something
        // with no path proves nothing.
        let mut targets: Vec<String> = price_db.known_commodities();
        targets.sort();
        targets.dedup();

        for target in targets.iter().take(3) {
            let Some(expected) =
                hledger_balances_with(&file, &["-X", target, "--infer-market-prices"])
            else {
                continue;
            };

            let rows = hledger_core::reports::balance_report_valued(
                &txns, None, None, None, target, &price_db,
            );
            let ours: BTreeMap<String, CommodityMap> = rows
                .iter()
                .map(|r| {
                    let m: CommodityMap = r
                        .amounts
                        .iter()
                        .filter_map(|a| {
                            a.quantity
                                .parse::<Decimal>()
                                .ok()
                                .filter(|q| !q.is_zero())
                                .map(|q| (a.commodity.clone(), q))
                        })
                        .collect();
                    (r.account.clone(), m)
                })
                .filter(|(_, m)| !m.is_empty())
                .collect();

            // Compare only the accounts hledger reports; our report includes
            // parent rows that its flat output does not.
            for (account, amounts) in &expected {
                let Some(got) = ours.get(account) else {
                    panic!("{}: -X {target} missing account {account}", file.display());
                };
                // hledger rounds a valued amount to the commodity's display
                // precision; we keep full precision so downstream sums don't
                // accumulate rounding. Compare at the precision it printed.
                for (commodity, theirs) in amounts {
                    let ours_q = got.get(commodity).copied().unwrap_or(Decimal::ZERO);
                    let dp = theirs.scale();
                    assert_eq!(
                        ours_q.round_dp(dp),
                        theirs.round_dp(dp),
                        "{}: -X {target} {account} {commodity} differs",
                        file.display()
                    );
                }
            }
        }
    }
}

/// Weekly, quarterly and yearly period columns must match hledger's.
///
/// Only the monthly interval had a check, and period boundaries are exactly
/// where an interval implementation goes wrong — ISO weeks especially.
#[test]
fn all_intervals_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }
    use hledger_core::periodic_report::{AccumulationMode, ReportInterval};

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };

        for (flag, interval) in [
            ("-W", ReportInterval::Weekly),
            ("-Q", ReportInterval::Quarterly),
            ("-Y", ReportInterval::Yearly),
        ] {
            let output = Command::new("hledger")
                .args(["-f", &file.to_string_lossy(), "bal", "--flat", "-N", flag, "-O", "csv"])
                .output()
                .expect("run hledger bal");
            if !output.status.success() {
                continue;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut lines = stdout.lines();
            let Some(header) = lines.next() else { continue };
            let periods: Vec<String> = parse_csv_row(header).into_iter().skip(1).collect();
            if periods.is_empty() {
                continue;
            }

            let expected: BTreeMap<String, Vec<CommodityMap>> = lines
                .filter_map(|line| {
                    let c = parse_csv_row(line);
                    (c.len() >= 2 && c[0] != "Total:").then(|| {
                        (c[0].clone(), c[1..].iter().map(|v| parse_amount_cell(v)).collect())
                    })
                })
                .collect();

            let report = hledger_core::periodic_report::periodic_balance_report(
                &txns,
                interval,
                AccumulationMode::Periodic,
                None,
                None,
                None,
                None,
                "",
                &hledger_core::price_db::PriceDb::default(),
            );

            assert_eq!(
                report.periods, periods,
                "{}: {flag} period columns differ",
                file.display()
            );

            for (account, cells) in &expected {
                let Some(row) = report.rows.iter().find(|r| &r.account == account) else {
                    continue;
                };
                for (i, cell) in cells.iter().enumerate() {
                    let got: CommodityMap = row.amounts[i]
                        .iter()
                        .filter_map(|a| {
                            a.quantity
                                .parse::<Decimal>()
                                .ok()
                                .filter(|q| !q.is_zero())
                                .map(|q| (a.commodity.clone(), q))
                        })
                        .collect();
                    assert_eq!(
                        &got, cell,
                        "{}: {flag} {account} period {} differs",
                        file.display(),
                        periods[i]
                    );
                }
            }
        }
    }
}

/// Date-range filtering must match hledger's `-b`/`-e`, whose end is exclusive.
#[test]
fn date_ranges_match_hledger() {
    if !hledger_available() {
        eprintln!("skipping: hledger not installed");
        return;
    }

    for file in all_fixtures() {
        let Some((txns, _)) = statement_inputs(&file) else { continue };
        let Some(first) = txns.iter().map(|t| t.date).min() else { continue };
        let Some(last) = txns.iter().map(|t| t.date).max() else { continue };
        let mid = first + chrono::Duration::days((last - first).num_days() / 2);

        for (begin, end) in [(first, mid), (mid, last), (first, last)] {
            let end_exclusive = end.succ_opt().unwrap();
            let Some(expected) = hledger_balances_with(
                &file,
                &["-b", &begin.to_string(), "-e", &end_exclusive.to_string()],
            ) else {
                continue;
            };

            let rows = hledger_core::reports::balance_report(
                &txns,
                None,
                Some(begin),
                Some(end),
            );
            let ours: BTreeMap<String, CommodityMap> = rows
                .iter()
                .map(|r| {
                    let m: CommodityMap = r
                        .amounts
                        .iter()
                        .filter_map(|a| {
                            a.quantity
                                .parse::<Decimal>()
                                .ok()
                                .filter(|q| !q.is_zero())
                                .map(|q| (a.commodity.clone(), q))
                        })
                        .collect();
                    (r.account.clone(), m)
                })
                .filter(|(_, m)| !m.is_empty())
                .collect();

            // balance_report gives inclusive parent rows; hledger's --flat is
            // exclusive. Roll its numbers up so this compares the date
            // filtering rather than that difference.
            for (account, amounts) in rolled_up(&expected) {
                let got = ours.get(&account).cloned().unwrap_or_default();
                assert_eq!(
                    got, amounts,
                    "{}: {begin}..{end} account {account} differs",
                    file.display()
                );
            }
        }
    }
}
