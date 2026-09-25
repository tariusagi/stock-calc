//! Portfolio data and on-disk persistence (JSON in the user's data folder).

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Holding {
    pub symbol: String,
    pub quantity: f64,
    pub cost_price: f64,
    pub current_price: f64,
    pub stop_loss: f64,
    pub target: f64,
}

impl Holding {
    pub fn total_cost(&self) -> f64 {
        self.quantity * self.cost_price
    }

    pub fn market_value(&self) -> f64 {
        self.quantity * self.current_price
    }

    pub fn unrealized(&self) -> f64 {
        if self.current_price > 0.0 {
            self.market_value() - self.total_cost()
        } else {
            0.0
        }
    }

    /// Stop loss distance from the cost price, in percent (negative = loss).
    pub fn stop_pct(&self) -> Option<f64> {
        (self.stop_loss > 0.0 && self.cost_price > 0.0)
            .then(|| (self.stop_loss - self.cost_price) / self.cost_price * 100.0)
    }

    /// Profit/loss if the stop loss is hit (negative = loss).
    pub fn total_loss(&self) -> f64 {
        if self.stop_loss > 0.0 {
            (self.stop_loss - self.cost_price) * self.quantity
        } else {
            0.0
        }
    }

    pub fn target_pct(&self) -> Option<f64> {
        (self.target > 0.0 && self.cost_price > 0.0)
            .then(|| (self.target - self.cost_price) / self.cost_price * 100.0)
    }

    /// Profit if the target is reached.
    pub fn total_gain(&self) -> f64 {
        if self.target > 0.0 {
            (self.target - self.cost_price) * self.quantity
        } else {
            0.0
        }
    }
}

/// Saved state of the stock calculator (`stockcalc.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CalculatorData {
    pub symbol: String,
    pub entry: f64,
    pub stop_loss: f64,
    pub risk_amount: f64,
    /// One entry per symbol, most recently modified first.
    pub history: Vec<HistoryEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct HistoryEntry {
    pub symbol: String,
    pub entry: f64,
    pub stop_loss: f64,
    pub risk_amount: f64,
    /// Last modification, in seconds since the Unix epoch.
    pub modified: u64,
}

impl HistoryEntry {
    pub fn sizing(&self) -> Result<Sizing, &'static str> {
        Sizing::compute(self.entry, self.stop_loss, self.risk_amount)
    }
}

impl CalculatorData {
    /// Records the current inputs under their symbol, updating the existing
    /// entry (and its timestamp) only if something changed.
    pub fn record_history(&mut self) {
        if self.symbol.is_empty() {
            return;
        }
        let fresh = HistoryEntry {
            symbol: self.symbol.clone(),
            entry: self.entry,
            stop_loss: self.stop_loss,
            risk_amount: self.risk_amount,
            modified: now_secs(),
        };
        if let Some(i) = self.history.iter().position(|e| e.symbol == fresh.symbol) {
            let e = &self.history[i];
            if (e.entry, e.stop_loss, e.risk_amount) == (fresh.entry, fresh.stop_loss, fresh.risk_amount) {
                return;
            }
            self.history.remove(i);
        }
        // Newest first; inserting at the front keeps that order even when
        // timestamps tie within the same second.
        self.history.insert(0, fresh);
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Position sizing: how many shares to buy so a stop-out loses at most
/// the risk amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizing {
    pub quantity: f64,
    pub risk_per_share: f64,
    /// Stop distance from entry in percent (negative).
    pub stop_pct: f64,
    pub position_size: f64,
    pub actual_risk: f64,
}

impl Sizing {
    pub fn compute(entry: f64, stop: f64, risk: f64) -> Result<Self, &'static str> {
        if entry <= 0.0 || risk <= 0.0 || stop <= 0.0 {
            return Err("Fill in entry, stop loss and risk amount.");
        }
        if stop >= entry {
            return Err("Stop loss must be below the entry price.");
        }
        let per_share = entry - stop;
        // Round down: buying more would risk more than the given amount.
        let quantity = (risk / per_share).floor();
        Ok(Self {
            quantity,
            risk_per_share: per_share,
            stop_pct: -per_share / entry * 100.0,
            position_size: quantity * entry,
            actual_risk: quantity * per_share,
        })
    }
}

/// The portfolio (`portfolio.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct PortfolioFile {
    pub holdings: Vec<Holding>,
}

/// `%APPDATA%\StockCalc`, overridable with the `STOCK_CALC_DIR` env var.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("STOCK_CALC_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("StockCalc")
}

pub fn portfolio_path() -> PathBuf {
    data_dir().join("portfolio.json")
}

pub fn calculator_path() -> PathBuf {
    data_dir().join("stockcalc.json")
}

pub fn load_portfolio() -> Result<PortfolioFile, String> {
    load_json(&portfolio_path())
}

pub fn save_portfolio(data: &PortfolioFile) -> io::Result<()> {
    save_json(&portfolio_path(), data)
}

pub fn load_calculator() -> Result<CalculatorData, String> {
    let path = calculator_path();
    if !path.exists() {
        // Older versions kept the calculator inputs inside portfolio.json.
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Legacy {
            calculator: CalculatorData,
        }
        return Ok(load_json::<Legacy>(&portfolio_path()).unwrap_or_default().calculator);
    }
    load_json(&path)
}

pub fn save_calculator(data: &CalculatorData) -> io::Result<()> {
    save_json(&calculator_path(), data)
}

/// Appends a position to the saved portfolio. A running portfolio window
/// notices the file change and reloads it.
pub fn append_holding(h: Holding) -> Result<(), String> {
    let mut data = load_portfolio()?;
    data.holdings.push(h);
    save_portfolio(&data).map_err(|e| format!("Could not save portfolio: {e}"))
}

/// Last modification time of the portfolio file, used to detect outside changes.
pub fn portfolio_modified() -> Option<SystemTime> {
    fs::metadata(portfolio_path()).and_then(|m| m.modified()).ok()
}

fn load_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            // Keep the unreadable file so auto-save can't destroy the user's data.
            let backup = path.with_extension("json.bak");
            let _ = fs::copy(path, &backup);
            format!("Could not read saved data ({e}). A backup was kept at {}", backup.display())
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("Could not open {}: {e}", path.display())),
    }
}

/// Writes to a temp file first and then renames, so a crash mid-write never
/// corrupts the saved data.
fn save_json<T: Serialize>(path: &Path, data: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_rounds_quantity_down() {
        let s = Sizing::compute(245.0, 230.0, 2_500_000.0).unwrap();
        assert_eq!(s.quantity, 166_666.0);
        assert_eq!(s.risk_per_share, 15.0);
        assert!(s.actual_risk <= 2_500_000.0);
        assert!(Sizing::compute(230.0, 245.0, 1000.0).is_err());
        assert!(Sizing::compute(0.0, 0.0, 1000.0).is_err());
    }

    #[test]
    fn history_keeps_one_entry_per_symbol_newest_first() {
        let mut d = CalculatorData { symbol: "AAA".into(), entry: 10.0, stop_loss: 9.0, risk_amount: 100.0, ..Default::default() };
        d.record_history();
        d.history[0].modified = 1; // pretend it is old
        d.symbol = "BBB".into();
        d.record_history();
        assert_eq!(d.history.len(), 2);
        assert_eq!(d.history[0].symbol, "BBB");

        // Unchanged inputs don't touch the timestamp.
        d.symbol = "AAA".into();
        d.record_history();
        assert_eq!(d.history[1].symbol, "AAA");
        assert_eq!(d.history[1].modified, 1);

        // Changed inputs update the existing entry and move it to the top.
        d.entry = 11.0;
        d.record_history();
        assert_eq!(d.history.len(), 2);
        assert_eq!((d.history[0].symbol.as_str(), d.history[0].entry), ("AAA", 11.0));
    }
}
