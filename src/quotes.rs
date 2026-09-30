//! Live prices for Vietnamese stocks (HOSE, HNX, UPCOM) from Yahoo Finance.
//!
//! Yahoo Finance lists Vietnamese equities with a `.VN` suffix and serves the
//! last traded price on its free chart endpoint — no API key needed:
//! `https://query1.finance.yahoo.com/v8/finance/chart/VCB.VN?interval=1d&range=1d`.

use serde_json::Value;
use std::time::Duration;
use ureq::Agent;

const BASE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const USER_AGENT: &str = "stock-calc (personal portfolio app)";
/// Kept short so a stalled network never freezes the refresh cycle for long.
const TIMEOUT: Duration = Duration::from_secs(8);

/// A fetched price: one quote per symbol that responded successfully.
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    /// Portfolio symbol, without the Yahoo suffix (`VCB`, not `VCB.VN`).
    pub symbol: String,
    pub price: f64,
}

/// `VCB` -> `VCB.VN`, how Yahoo Finance spells Vietnamese tickers.
pub fn yahoo_symbol(symbol: &str) -> String {
    format!("{symbol}.VN")
}

/// Fetches the last traded price of each symbol, one request per symbol.
/// Symbols that fail (unknown on Yahoo, no network, ...) are left out of the
/// result; the caller keeps their last known price.
pub fn fetch_prices(symbols: &[String]) -> Vec<Quote> {
    let agent: Agent = Agent::config_builder().timeout_global(Some(TIMEOUT)).build().into();
    symbols.iter().filter_map(|s| fetch_one(&agent, s)).collect()
}

fn fetch_one(agent: &Agent, symbol: &str) -> Option<Quote> {
    let url = format!("{BASE_URL}/{}?interval=1d&range=1d", yahoo_symbol(symbol));
    let text = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let price = parse_price(&text)?;
    Some(Quote { symbol: symbol.to_owned(), price })
}

/// Extracts `chart.result[0].meta.regularMarketPrice` from a chart response.
fn parse_price(json: &str) -> Option<f64> {
    let v: Value = serde_json::from_str(json).ok()?;
    v["chart"]["result"][0]["meta"]["regularMarketPrice"].as_f64().filter(|p| *p > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_vietnamese_symbols() {
        assert_eq!(yahoo_symbol("VCB"), "VCB.VN");
    }

    #[test]
    fn parses_chart_response() {
        let json = r#"{"chart":{"result":[{"meta":{"currency":"VND","symbol":"VCB.VN",
            "regularMarketPrice":58000.0}}],"error":null}}"#;
        assert_eq!(parse_price(json), Some(58000.0));
    }

    #[test]
    fn rejects_bad_responses() {
        assert_eq!(parse_price("{}"), None);
        assert_eq!(parse_price("not json"), None);
        assert_eq!(parse_price(r#"{"chart":{"error":{"code":"Not Found"}}}"#), None);
        assert_eq!(parse_price(r#"{"chart":{"result":[]}}"#), None);
        assert_eq!(parse_price(r#"{"chart":{"result":[{"meta":{"regularMarketPrice":0.0}}]}}"#), None);
    }

    /// Real network call; run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore = "network"]
    fn fetches_live_price() {
        let quotes = fetch_prices(&["VCB".to_owned(), "NO_SUCH_TICKER".to_owned()]);
        assert_eq!(quotes.len(), 1);
        assert!(quotes[0].price > 0.0);
        assert_eq!(quotes[0].symbol, "VCB");
    }
}