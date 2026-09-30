//! Importing positions from broker portfolio files.
//!
//! Currently supported: the SSI iBoard portfolio export (`.xlsx`). Its table
//! has bilingual headers (`Mã CK\n(Symbol)`), numbers as comma-grouped text
//! (`"1,234,567"`), `-` for missing values, a `Tổng/Total` row and footnotes.

use calamine::{Data, Reader, Xlsx};
use std::path::Path;
use crate::model::Holding;

/// Brokers whose portfolio files can be imported. Each broker has its own
/// export format; add new ones here and match them in [`import`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Broker {
    /// SSI iBoard portfolio export (.xlsx).
    Ssi,
}

/// The brokers offered in the import menu, in display order.
pub const BROKERS: &[Broker] = &[Broker::Ssi];

impl Broker {
    /// Label shown in the import menu.
    pub fn label(self) -> &'static str {
        match self {
            Broker::Ssi => "SSI — iBoard export (.xlsx)",
        }
    }

    /// File-picker filter for this broker's exports.
    pub fn file_filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Broker::Ssi => ("Excel files", &["xlsx"]),
        }
    }
}

/// Reads positions from a broker portfolio export.
pub fn import(path: &Path, broker: Broker) -> Result<Vec<Holding>, String> {
    match broker {
        Broker::Ssi => import_ssi(path),
    }
}

/// Column indexes of the four values we import.
#[derive(Clone, Copy)]
struct Columns {
    symbol: usize,
    quantity: usize,
    cost: usize,
    current: usize,
}

/// Reads positions from an SSI iBoard portfolio export.
///
/// Only the raw position data is taken — symbol, quantity, average cost and
/// market price. Everything the app computes itself (total cost, P/L, ...) is
/// ignored, and stop loss / target start empty for the user to set.
pub fn import_ssi(path: &Path) -> Result<Vec<Holding>, String> {
    let mut wb: Xlsx<_> =
        calamine::open_workbook(path).map_err(|e| format!("Could not open the file: {e}"))?;
    let range = wb
        .worksheet_range_at(0)
        .ok_or("The workbook has no sheets")?
        .map_err(|e| format!("Could not read the sheet: {e}"))?;

    // The header row sits below the title and account number lines; find it
    // by its bilingual column labels.
    let header = range
        .rows()
        .enumerate()
        .find_map(|(i, row)| find_columns(row).map(|cols| (i, cols)))
        .ok_or("Could not find the table header (Mã CK/Symbol, Giá vốn/Avg Cost, ...)")?;

    let mut out = Vec::new();
    for row in range.rows().skip(header.0 + 1) {
        let symbol = cell_text(row.get(header.1.symbol))
            .map(|s| s.trim().to_uppercase())
            .unwrap_or_default();
        // Empty symbols cover the totals row ("Tổng/Total" sits in the first
        // column) and the footnotes below the table.
        if symbol.is_empty() || symbol.starts_with("TỔNG") || symbol.eq_ignore_ascii_case("TOTAL") {
            continue;
        }
        out.push(Holding {
            symbol,
            quantity: cell_num(row.get(header.1.quantity)).unwrap_or(0.0),
            cost_price: cell_num(row.get(header.1.cost)).unwrap_or(0.0),
            current_price: cell_num(row.get(header.1.current)).unwrap_or(0.0),
            // Stop loss, target and pinned are the user's own, not broker data.
            ..Default::default()
        });
    }
    if out.is_empty() {
        return Err("No positions found in the file.".to_owned());
    }
    Ok(out)
}

/// Locates our columns in a candidate header row, accepting either the
/// Vietnamese or the English part of each bilingual label.
fn find_columns(row: &[Data]) -> Option<Columns> {
    let texts: Vec<String> = row
        .iter()
        .map(|c| cell_text(Some(c)).unwrap_or_default().replace('\n', " ").to_lowercase())
        .collect();
    let find = |pred: &dyn Fn(&str) -> bool| texts.iter().position(|t| pred(t));
    Some(Columns {
        symbol: find(&|t| t.contains("mã ck") || t.contains("(symbol)"))?,
        quantity: find(&|t| t.contains("tổng khối lượng") || t.contains("total volume"))?,
        cost: find(&|t| t.contains("giá vốn") || t.contains("avg cost"))?,
        current: find(&|t| t.contains("giá thị trường") || t.contains("market price"))?,
    })
}

/// The text of a cell, if it holds one.
fn cell_text(cell: Option<&Data>) -> Option<String> {
    match cell? {
        Data::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Reads a cell as a number. The export stores numbers as text like
/// `"1,234,567"`, `"-13.88%"` or plain `-` for missing values; real numeric
/// cells arrive as floats.
fn cell_num(cell: Option<&Data>) -> Option<f64> {
    match cell? {
        Data::Float(v) => Some(*v),
        Data::Int(v) => Some(*v as f64),
        Data::String(s) => {
            let cleaned: String =
                s.chars().filter(|&c| c.is_ascii_digit() || c == '.' || c == '-').collect();
            cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;
    use std::path::PathBuf;

    /// Writes a workbook laid out like the SSI iBoard export and returns its
    /// path. Numbers are comma-grouped text, `-` marks a missing market price
    /// (e.g. a suspended stock), and the totals row plus footnotes close it.
    fn sample_file(name: &str) -> PathBuf {
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        ws.write(0, 0, "DANH MỤC CHỨNG KHOÁN/ PORFOLIO").unwrap();
        ws.write(1, 2, "Số tài khoản/ Account number:").unwrap();
        ws.write(1, 4, "003C1151826").unwrap();
        // Header row, bilingual labels with line breaks like the real export.
        ws.write(2, 0, "STT\n").unwrap();
        ws.write(2, 1, "Mã CK\n(Symbol)").unwrap();
        ws.write(2, 2, "Tổng khối lượng\n(Total Volume)").unwrap();
        ws.write(2, 3, "Khối lượng giao dịch\n(Tradeable quantity)").unwrap();
        ws.write(2, 4, "Giá vốn\n(Avg Cost)").unwrap();
        ws.write(2, 5, "Giá thị trường\n(Market price)").unwrap();
        ws.write(2, 6, "Vốn\n(Cost Value)").unwrap();
        ws.write(2, 7, "Lãi/ Lỗ\n(Profit/Loss)").unwrap();

        ws.write(3, 0, 1).unwrap();
        ws.write(3, 1, "AAA").unwrap();
        ws.write(3, 2, "5,000").unwrap();
        ws.write(3, 3, "5,000").unwrap();
        ws.write(3, 4, "8,395").unwrap();
        ws.write(3, 5, "7,230").unwrap();
        ws.write(3, 6, "41,975,000").unwrap();
        ws.write(3, 7, "-5,825,000").unwrap();

        // Market price missing ("-"), everything else present.
        ws.write(4, 0, 2).unwrap();
        ws.write(4, 1, "bii").unwrap();
        ws.write(4, 2, "20,000").unwrap();
        ws.write(4, 4, "900").unwrap();
        ws.write(4, 5, "-").unwrap();

        // Totals row: "Tổng/Total" in the first column, no symbol.
        ws.write(5, 0, "Tổng/Total").unwrap();
        ws.write(5, 6, "51,825,000").unwrap();
        // Footnotes.
        ws.write(7, 0, "* Dữ liệu xuất tối đa 1.000 dòng/Data export max 1.000 rows").unwrap();

        let path = std::env::temp_dir().join(format!("stock_calc_import_{name}.xlsx"));
        wb.save(&path).unwrap();
        path
    }

    #[test]
    fn imports_ssi_export() {
        let path = sample_file("ok");
        let holdings = import_ssi(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(holdings.len(), 2);
        assert_eq!(holdings[0].symbol, "AAA");
        assert_eq!(holdings[0].quantity, 5_000.0);
        assert_eq!(holdings[0].cost_price, 8_395.0);
        assert_eq!(holdings[0].current_price, 7_230.0);
        // Stop loss and target are the user's plan, not broker data.
        assert_eq!(holdings[0].stop_loss, 0.0);
        assert_eq!(holdings[0].target, 0.0);
        // Lowercase symbol is uppercased; the "-" market price reads as zero
        // (the live Yahoo refresh fills it in).
        assert_eq!(holdings[1].symbol, "BII");
        assert_eq!(holdings[1].quantity, 20_000.0);
        assert_eq!(holdings[1].cost_price, 900.0);
        assert_eq!(holdings[1].current_price, 0.0);
    }

    #[test]
    fn rejects_files_without_the_table() {
        let path = std::env::temp_dir().join("stock_calc_import_unrelated.xlsx");
        let mut wb = Workbook::new();
        wb.add_worksheet().write(0, 0, "Something else entirely").unwrap();
        wb.save(&path).unwrap();
        let err = import_ssi(&path).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("Could not find the table header"));
    }

    #[test]
    fn parses_export_numbers() {
        let d = |s: &str| Data::String(s.to_owned());
        assert_eq!(cell_num(Some(&d("1,234,567"))), Some(1_234_567.0));
        assert_eq!(cell_num(Some(&d("-5,825,000"))), Some(-5_825_000.0));
        assert_eq!(cell_num(Some(&d("-13.88%"))), Some(-13.88));
        assert_eq!(cell_num(Some(&d("-"))), None);
        assert_eq!(cell_num(Some(&d(""))), None);
        assert_eq!(cell_num(Some(&Data::Float(12.5))), Some(12.5));
        assert_eq!(cell_num(Some(&Data::Int(3))), Some(3.0));
        assert_eq!(cell_num(None), None);
    }

    /// A real SSI iBoard export dropped next to the repo, if present:
    /// `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs the SSI export in the repo root"]
    fn imports_the_real_export() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("SSI_iBoard_Portfolio_003C1151826.xlsx");
        if !path.exists() {
            return;
        }
        let holdings = import_ssi(&path).unwrap();
        assert!(!holdings.is_empty());
        assert!(holdings.iter().all(|h| !h.symbol.is_empty()));
        assert!(holdings.iter().all(|h| h.quantity > 0.0));
        assert!(holdings.iter().all(|h| h.stop_loss == 0.0 && h.target == 0.0));
    }
}