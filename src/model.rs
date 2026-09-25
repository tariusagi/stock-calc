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

/// Saved inputs of the stock calculator (`stockcalc.json`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CalculatorData {
    pub symbol: String,
    pub entry: f64,
    pub stop_loss: f64,
    pub risk_amount: f64,
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
