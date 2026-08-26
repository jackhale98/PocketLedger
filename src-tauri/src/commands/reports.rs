use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use hledger_core::reports;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportParams {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub account_filter: Option<String>,
    pub target_commodity: Option<String>,
    /// hledger query string filtering postings (acct:, desc:, amt:, ...).
    #[serde(default)]
    pub query: Option<String>,
    /// Extend reports with transactions materialized from periodic rules.
    #[serde(default)]
    pub forecast: Option<bool>,
}

/// Parse the params' query string, erroring loudly on bad syntax.
pub fn parse_query_param(params: &ReportParams) -> Result<Option<hledger_core::query::Query>, String> {
    match params.query.as_deref() {
        Some(q) if !q.trim().is_empty() => {
            let parsed = hledger_core::query::parse_query(q)?;
            Ok(Some(parsed))
        }
        _ => Ok(None),
    }
}

/// The transaction set a report should run over: real transactions, plus
/// forecast transactions up to date_to (or six months past the last real
/// transaction) when requested.
/// The transaction list every report should work from: the journal's own
/// transactions, optionally extended with the forecast, then narrowed by the
/// query. Reports that build this list by hand silently ignore the filter the
/// user set, which is the inconsistency this exists to prevent.
pub fn transactions_for(
    loaded: &super::journal::LoadedJournal,
    params: &ReportParams,
) -> Result<Vec<hledger_core::balance::ResolvedTransaction>, String> {
    let real: Vec<_> = loaded.ledger.transactions().cloned().collect();
    let txns = if params.forecast.unwrap_or(false) {
        hledger_core::forecast::with_forecast(&loaded.journal, &real, parse_date(&params.date_to))
    } else {
        real
    };
    match parse_query_param(params)? {
        Some(q) => Ok(hledger_core::query::retain_matching_postings(&txns, &q)),
        None => Ok(txns),
    }
}

pub fn parse_date(s: &Option<String>) -> Option<chrono::NaiveDate> {
    s.as_ref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// Resolve the valuation target: the requested commodity if it actually
/// exists in the journal, else the journal's most-used commodity. The old
/// hardcoded "$" default sent every non-$ journal down a garbage path.
pub fn resolve_target_commodity(
    loaded: &super::journal::LoadedJournal,
    requested: Option<&str>,
) -> String {
    if let Some(req) = requested {
        if !req.is_empty() {
            let exists = loaded.ledger.transactions().any(|t| {
                t.postings
                    .iter()
                    .any(|p| p.amount.amounts.contains_key(req))
            });
            if exists {
                return req.to_string();
            }
        }
    }
    loaded.ledger.primary_commodity().unwrap_or_default()
}

#[tauri::command]
pub async fn balance_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns = transactions_for(loaded, &params)?;
    // Valued like every other report, so the currency the user picked applies
    // here too rather than only in the charts.
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    Ok(reports::balance_report_valued(
        &txns,
        params.account_filter.as_deref(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        &commodity,
        loaded.ledger.price_db(),
    ))
}

#[tauri::command]
pub async fn register_report(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::RegisterRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let txns = transactions_for(loaded, &params)?;
    Ok(reports::register_report(
        &txns,
        &account,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn periodic_balance(
    interval: String,
    mode: Option<String>,
    depth: Option<usize>,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<hledger_core::periodic_report::PeriodicBalanceReport, String> {
    use hledger_core::periodic_report::{AccumulationMode, ReportInterval};

    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let interval = match interval.as_str() {
        "weekly" | "W" => ReportInterval::Weekly,
        "monthly" | "M" => ReportInterval::Monthly,
        "quarterly" | "Q" => ReportInterval::Quarterly,
        "yearly" | "Y" => ReportInterval::Yearly,
        other => return Err(format!("unknown interval '{}'", other)),
    };
    let mode = match mode.as_deref() {
        None | Some("periodic") => AccumulationMode::Periodic,
        Some("cumulative") => AccumulationMode::Cumulative,
        Some("historical") => AccumulationMode::Historical,
        Some(other) => return Err(format!("unknown accumulation mode '{}'", other)),
    };

    // The query is still passed through: transactions_for has already applied
    // it, but the report also reads `depth:` off it.
    let query = parse_query_param(&params)?;
    let txns = transactions_for(loaded, &params)?;
    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());

    Ok(hledger_core::periodic_report::periodic_balance_report(
        &txns,
        interval,
        mode,
        depth,
        query.as_ref(),
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        &commodity,
        loaded.ledger.price_db(),
    ))
}

#[tauri::command]
pub async fn balance_sheet_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::balance_sheet(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn income_statement_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::income_statement(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn cash_flow_report(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::FinancialStatement, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::cash_flow(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn net_worth_series(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::net_worth_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn account_balance_series(
    account: String,
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::TimeSeriesPoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::account_series(
        &txns,
        loaded.ledger.price_db(),
        &account,
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn income_expense_chart(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::IncomeExpensePoint>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::income_expense_series(
        &txns,
        loaded.ledger.classifier(),
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
    ))
}

#[tauri::command]
pub async fn expense_breakdown_chart(
    params: ReportParams,
    parent_prefix: Option<String>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::PieSlice>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::expense_breakdown(
        &txns,
        loaded.ledger.price_db(),
        &commodity,
        parse_date(&params.date_from),
        parse_date(&params.date_to),
        parent_prefix.as_deref(),
    ))
}

/// Info the UI needs to label valued charts honestly: which commodity the
/// numbers are in, and which commodities could NOT be valued (and are
/// therefore excluded from single-number charts).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuationInfo {
    pub target_commodity: String,
    pub unconvertible: Vec<String>,
}

#[tauri::command]
pub async fn valuation_info(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<ValuationInfo, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let commodity = resolve_target_commodity(loaded, params.target_commodity.as_deref());
    let txns = transactions_for(loaded, &params)?;
    let date = parse_date(&params.date_to)
        .or_else(|| txns.last().map(|t| t.date))
        .unwrap_or_else(|| chrono::Local::now().date_naive());

    let unconvertible =
        reports::unconvertible_commodities(&txns, &commodity, loaded.ledger.price_db(), date);

    Ok(ValuationInfo {
        target_commodity: commodity,
        unconvertible,
    })
}

#[tauri::command]
pub async fn list_accounts_with_balances(
    params: Option<ReportParams>,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<reports::BalanceRow>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let effective = params.clone().unwrap_or_default();
    let txns = transactions_for(loaded, &effective)?;

    if let Some(params) = params {
        if let Some(target) = params.target_commodity.as_deref() {
            if !target.is_empty() {
                return Ok(reports::balance_report_valued(
                    &txns,
                    params.account_filter.as_deref(),
                    parse_date(&params.date_from),
                    parse_date(&params.date_to),
                    target,
                    loaded.ledger.price_db(),
                ));
            }
        }
    }

    Ok(reports::balance_report(&txns, None, None, None))
}

/// Price history per commodity pair, for a chart of what each is worth over
/// time.
#[tauri::command]
pub async fn commodity_prices(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<hledger_core::price_db::PriceSeries>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    Ok(loaded.ledger.price_db().series())
}

#[tauri::command]
pub async fn journal_statistics(
    params: ReportParams,
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<reports::JournalStatistics, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;
    let txns = transactions_for(loaded, &params)?;
    Ok(reports::journal_statistics(&txns))
}

#[tauri::command]
pub async fn list_commodities(
    state: State<'_, Mutex<crate::AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let loaded = app_state.journal.as_ref().ok_or("No journal loaded")?;

    let mut commodities = std::collections::BTreeSet::new();
    for txn in loaded.ledger.transactions() {
        for posting in &txn.postings {
            for commodity in posting.amount.amounts.keys() {
                if !commodity.is_empty() {
                    commodities.insert(commodity.clone());
                }
            }
        }
    }
    Ok(commodities.into_iter().collect())
}
