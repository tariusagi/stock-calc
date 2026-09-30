//! portfolio.exe — the stock portfolio manager.

#![windows_subsystem = "windows"] // no console window on Windows

use std::{
    cell::Cell,
    cmp::Ordering,
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

use chrono::{DateTime, Local};
use eframe::egui::{self, Align, Button, Color32, Layout, Margin, RichText, Stroke, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use stock_calc::format::{
    fmt_input, fmt_int, fmt_signed_pct, num_input, num_input_bg, parse_or_zero, symbol_input_colored,
};
use stock_calc::import;
use stock_calc::instance::{self, App};
use stock_calc::model::{self, Holding, PortfolioFile};
use stock_calc::quotes::{self, Quote};
use stock_calc::theme::*;

/// How often the current-price column is refreshed from Yahoo Finance.
const QUOTE_INTERVAL: Duration = Duration::from_secs(10);

fn main() -> eframe::Result {
    if !instance::claim(App::Portfolio) {
        instance::hand_over(App::Portfolio);
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(App::Portfolio.title())
        .with_inner_size([1400.0, 740.0])
        .with_min_inner_size([900.0, 480.0]);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Stock Portfolio", options, Box::new(|cc| Ok(Box::new(PortfolioApp::new(cc)))))
}

// ---------------------------------------------------------------------------
// Editable state (text buffers so partially typed numbers survive frames)

#[derive(Default)]
struct RowUi {
    symbol: String,
    quantity: String,
    cost_price: String,
    current_price: String,
    stop_loss: String,
    target: String,
    pinned: bool,
}

impl RowUi {
    fn from_holding(h: &Holding) -> Self {
        Self {
            symbol: h.symbol.clone(),
            quantity: fmt_input(h.quantity),
            cost_price: fmt_input(h.cost_price),
            current_price: fmt_input(h.current_price),
            stop_loss: fmt_input(h.stop_loss),
            target: fmt_input(h.target),
            pinned: h.pinned,
        }
    }

    fn to_holding(&self) -> Holding {
        Holding {
            symbol: self.symbol.clone(),
            quantity: parse_or_zero(&self.quantity),
            cost_price: parse_or_zero(&self.cost_price),
            current_price: parse_or_zero(&self.current_price),
            stop_loss: parse_or_zero(&self.stop_loss),
            target: parse_or_zero(&self.target),
            pinned: self.pinned,
        }
    }
}

enum Status {
    Saved,
    Error(String),
}

// ---------------------------------------------------------------------------
// Row coloring

/// What the table is sorted by.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SortKey {
    Symbol,
    TotalCost,
    Pl,
    PlPct,
}

/// Unrealized P/L relative to cost, if it can be computed.
fn pl_pct(h: &Holding) -> Option<f64> {
    (h.total_cost() > 0.0 && h.current_price > 0.0)
        .then(|| h.unrealized() / h.total_cost() * 100.0)
}

/// Display order of the table: pinned rows first, then by `key` (ascending,
/// or descending when `desc`); rows without a symbol always go last. Returns
/// indexes into `rows`/`holdings`.
fn sort_order(rows: &[RowUi], holdings: &[Holding], key: SortKey, desc: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (&rows[a], &rows[b]);
        let (ha, hb) = (&holdings[a], &holdings[b]);
        let mut ord = match key {
            SortKey::Symbol => ha.symbol.cmp(&hb.symbol),
            SortKey::TotalCost => ha.total_cost().total_cmp(&hb.total_cost()),
            SortKey::Pl => ha.unrealized().total_cmp(&hb.unrealized()),
            SortKey::PlPct => match (pl_pct(ha), pl_pct(hb)) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        };
        if desc {
            ord = ord.reverse();
        }
        // Rows without a symbol sit at the bottom in either direction.
        let ord = match (ra.symbol.is_empty(), rb.symbol.is_empty()) {
            (true, true) | (false, false) => ord,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
        };
        // Pinned rows go first, sorted like everything else.
        rb.pinned.cmp(&ra.pinned).then_with(|| ord)
    });
    order
}

/// Symbol color by current P/L: red for a loss, green for a gain, normal
/// text color otherwise.
fn pl_color(pl: f64) -> Color32 {
    if pl > 0.0 {
        GREEN
    } else if pl < 0.0 {
        RED
    } else {
        TEXT
    }
}

/// How urgently the stop loss should blink: `true` = rapidly (the price has
/// dropped below the stop), `false` = slowly (the price is within 0.5% above
/// it), `None` = comfortably away from the stop.
fn stop_alert(h: &Holding) -> Option<bool> {
    if h.stop_loss <= 0.0 || h.current_price <= 0.0 {
        return None;
    }
    let distance = (h.current_price - h.stop_loss) / h.stop_loss * 100.0;
    if distance < 0.0 {
        Some(true)
    } else if distance <= 0.5 {
        Some(false)
    } else {
        None
    }
}

/// Seconds per pulse of the stop-loss blink: a slow warning pulse when the
/// price is near the stop, a faster red alarm once it is below it.
const SLOW_BLINK: f64 = 2.0;
const FAST_BLINK: f64 = 0.8;

/// Pulsing between soft and full strength, driven by the frame time.
fn blink_color(ui: &Ui, period: f64, base: Color32) -> Color32 {
    let phase = (ui.input(|i| i.time) / period).fract();
    let pulse = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * phase).cos();
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), ((0.45 + 0.55 * pulse) * 255.0) as u8)
}

struct PortfolioApp {
    rows: Vec<RowUi>,
    last_saved: PortfolioFile,
    /// Modification time of the portfolio file as of our last read/write;
    /// a different time means another program (the calculator) changed it.
    known_mtime: Option<SystemTime>,
    status: Status,
    confirm_delete: Option<usize>,
    /// Row whose symbol field should grab keyboard focus next frame.
    focus_row: Option<usize>,
    /// Live prices from Yahoo Finance: a background thread fetches the whole
    /// portfolio and sends the quotes over this channel.
    quotes_tx: mpsc::Sender<Vec<Quote>>,
    quotes_rx: mpsc::Receiver<Vec<Quote>>,
    /// Next moment a refresh should start (start-up + every 10 s).
    next_quote_fetch: Instant,
    /// A fetch thread is out; a new one starts once it has reported back.
    quote_fetch_running: bool,
    /// When a price refresh last delivered quotes, shown in the status bar.
    last_quote_update: Option<DateTime<Local>>,
    /// Positions read from a broker file, waiting for the user to decide
    /// whether they are added to or replace the current ones.
    import: Option<Vec<Holding>>,
    /// How the table is displayed (view setting, not saved).
    sort_key: SortKey,
    sort_desc: bool,
}

impl PortfolioApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let (quotes_tx, quotes_rx) = mpsc::channel();
        let mut app = Self {
            rows: Vec::new(),
            last_saved: PortfolioFile::default(),
            known_mtime: None,
            status: Status::Saved,
            confirm_delete: None,
            focus_row: None,
            quotes_tx,
            quotes_rx,
            // The first frame starts the initial fetch.
            next_quote_fetch: Instant::now(),
            quote_fetch_running: false,
            last_quote_update: None,
            import: None,
            sort_key: SortKey::Symbol,
            sort_desc: false,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.known_mtime = model::portfolio_modified();
        match model::load_portfolio() {
            Ok(data) => {
                self.rows = data.holdings.iter().map(RowUi::from_holding).collect();
                self.last_saved = data;
                self.status = Status::Saved;
            }
            Err(e) => self.status = Status::Error(e),
        }
        self.confirm_delete = None;
    }

    fn snapshot(&self) -> PortfolioFile {
        PortfolioFile { holdings: self.rows.iter().map(RowUi::to_holding).collect() }
    }

    fn autosave(&mut self) {
        let snap = self.snapshot();
        if snap == self.last_saved {
            return;
        }
        match model::save_portfolio(&snap) {
            Ok(()) => self.status = Status::Saved,
            Err(e) => self.status = Status::Error(format!("Could not save: {e}")),
        }
        self.known_mtime = model::portfolio_modified();
        // Remember it even on failure to avoid retrying every frame;
        // the next edit will try again.
        self.last_saved = snap;
    }

    fn open_calculator(&mut self) {
        if let Err(e) = instance::open(App::Calculator) {
            self.status = Status::Error(e);
        }
    }

    /// Opens a file picker for the chosen broker's export format; a picked
    /// file is read right away and held until the user decides how to merge.
    fn pick_import_file(&mut self, broker: import::Broker) {
        let (filter_name, extensions) = broker.file_filter();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import portfolio")
            .add_filter(filter_name, extensions)
            .pick_file()
        else {
            return; // dialog cancelled
        };
        match import::import(&path, broker) {
            Ok(holdings) => self.import = Some(holdings),
            Err(e) => self.status = Status::Error(e),
        }
    }

    /// Asks whether the imported positions should be added to the portfolio
    /// or replace it entirely.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(holdings) = self.import.clone() else { return };
        let n = holdings.len();
        #[derive(Clone, Copy, PartialEq)]
        enum Choice {
            Add,
            Replace,
            Cancel,
        }
        let mut choice: Option<Choice> = None;

        egui::Window::new("Import portfolio")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(window_frame())
            .show(ctx, |ui| {
                ui.label(RichText::new(format!(
                    "Found {n} {} in the file.",
                    if n == 1 { "position" } else { "positions" }
                ))
                .size(15.0));
                ui.add_space(4.0);
                ui.label("Add them to your portfolio, or replace the current ones?");
                ui.label(
                    RichText::new("Adding merges rows with the same symbol: quantities add up and the cost price is averaged.")
                        .size(12.5)
                        .color(MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let add = Button::new(
                        RichText::new("Add to portfolio").font(semibold(14.0)).color(Color32::WHITE),
                    )
                    .fill(ACCENT)
                    .corner_radius(8);
                    if ui.add(add).clicked() {
                        choice = Some(Choice::Add);
                    }
                    let replace = Button::new(
                        RichText::new("Replace portfolio").font(semibold(14.0)).color(Color32::WHITE),
                    )
                    .fill(RED)
                    .corner_radius(8);
                    if ui.add(replace).clicked() {
                        choice = Some(Choice::Replace);
                    }
                    if ui.add(Button::new("Cancel").corner_radius(8)).clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });

        match choice {
            Some(Choice::Add) => {
                merge_rows(&mut self.rows, &holdings);
                self.import = None;
            }
            Some(Choice::Replace) => {
                self.rows = holdings.iter().map(RowUi::from_holding).collect();
                self.confirm_delete = None;
                self.import = None;
            }
            _ => {}
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.import = None;
        }
    }

    /// Starts a background thread that fetches the current price of every
    /// stock in the table from Yahoo Finance and reports back on the channel.
    fn spawn_quote_fetch(&mut self, ctx: egui::Context) {
        let mut symbols: Vec<String> = self
            .rows
            .iter()
            .map(|r| r.symbol.clone())
            .filter(|s| !s.is_empty())
            .collect();
        symbols.sort();
        symbols.dedup();
        if symbols.is_empty() {
            return;
        }
        self.quote_fetch_running = true;
        let tx = self.quotes_tx.clone();
        std::thread::spawn(move || {
            let quotes = quotes::fetch_prices(&symbols);
            // Fails only when the app has already closed.
            let _ = tx.send(quotes);
            ctx.request_repaint();
        });
    }

    /// Applies fetched live prices to the current-price column and saves the
    /// update to disk. Symbols the source doesn't know keep their saved price.
    fn apply_prices(&mut self, quotes: Vec<Quote>) {
        // An empty batch means every fetch failed; keep the old timestamp so
        // the status bar keeps showing the last time prices were real.
        if !quotes.is_empty() {
            self.last_quote_update = Some(Local::now());
        }
        let mut updated = false;
        for q in quotes {
            for row in &mut self.rows {
                if row.symbol == q.symbol {
                    let text = fmt_input(q.price);
                    if row.current_price != text {
                        row.current_price = text;
                        updated = true;
                    }
                }
            }
        }
        if updated {
            self.autosave();
        }
    }
}

impl eframe::App for PortfolioApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Pick up positions added by the Stock Calculator.
        if model::portfolio_modified() != self.known_mtime {
            self.reload();
        }

        // Refresh live prices: once at start up, then every 10 s until closing.
        if Instant::now() >= self.next_quote_fetch {
            self.next_quote_fetch = Instant::now() + QUOTE_INTERVAL;
            if !self.quote_fetch_running {
                self.spawn_quote_fetch(ui.ctx().clone());
            }
        }
        while let Ok(quotes) = self.quotes_rx.try_recv() {
            self.quote_fetch_running = false;
            self.apply_prices(quotes);
        }
        // Keep frames ticking while idle so the refresh timer fires — and
        // fast enough that an alerting stop loss blinks smoothly.
        let blinking = self.rows.iter().any(|r| stop_alert(&r.to_holding()).is_some());
        ui.ctx().request_repaint_after(if blinking {
            Duration::from_millis(33)
        } else {
            Duration::from_secs(1)
        });

        self.header(ui);
        self.status_bar(ui);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 16)))
            .show(ui, |ui| {
                let holdings: Vec<Holding> = self.rows.iter().map(RowUi::to_holding).collect();
                summary_cards(ui, &holdings);
                ui.add_space(16.0);
                self.table_card(ui, &holdings);
            });

        self.delete_dialog(ui.ctx());
        self.import_dialog(ui.ctx());
        self.autosave();
    }
}

// ---------------------------------------------------------------------------
// Sections

impl PortfolioApp {
    fn header(&mut self, ui: &mut Ui) {
        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(ACCENT).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📈").size(26.0).color(Color32::WHITE));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("My Portfolio").font(semibold(22.0)).color(Color32::WHITE));
                        let n = self.rows.len();
                        let label = if n == 1 { "1 position".to_owned() } else { format!("{n} positions") };
                        ui.label(RichText::new(label).size(13.0).color(ACCENT_MUTED));
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let calc_btn = Button::new(
                            RichText::new("🖩  Stock Calculator").font(semibold(15.0)).color(ACCENT),
                        )
                        .fill(Color32::WHITE)
                        .corner_radius(10)
                        .min_size(Vec2::new(0.0, 36.0));
                        if ui.add(calc_btn).clicked() {
                            self.open_calculator();
                        }

                        let add_btn = Button::new(
                            RichText::new("➕  Add stock").font(semibold(15.0)).color(Color32::WHITE),
                        )
                        .fill(ACCENT_DARK)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x81, 0x8C, 0xF8)))
                        .corner_radius(10)
                        .min_size(Vec2::new(0.0, 36.0));
                        if ui.add(add_btn).clicked() {
                            self.rows.push(RowUi::default());
                            self.focus_row = Some(self.rows.len() - 1);
                        }

                        let import_btn = Button::new(
                            RichText::new("📂  Import").font(semibold(15.0)).color(ACCENT),
                        )
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(10)
                        .min_size(Vec2::new(0.0, 36.0));
                        let import_resp = ui.add(import_btn);
                        // Let the user pick which broker's file to import;
                        // each broker has its own export format.
                        egui::Popup::from_toggle_button_response(&import_resp)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_min_width(210.0);
                                ui.label(
                                    RichText::new("Import from broker").size(12.0).color(MUTED),
                                );
                                ui.separator();
                                for broker in import::BROKERS {
                                    if ui.selectable_label(false, broker.label()).clicked() {
                                        self.pick_import_file(*broker);
                                        ui.close();
                                    }
                                }
                            });
                    });
                });
            });
    }

    fn status_bar(&self, ui: &mut Ui) {
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(20, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.status {
                        Status::Saved => {
                            ui.label(RichText::new("●").color(GREEN).size(11.0));
                            ui.label(
                                RichText::new(format!(
                                    "All changes saved automatically to {}",
                                    model::portfolio_path().display()
                                ))
                                .size(12.5)
                                .color(MUTED),
                            );
                        }
                        Status::Error(e) => {
                            ui.label(RichText::new("●").color(RED).size(11.0));
                            ui.label(RichText::new(e).size(12.5).color(RED));
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let text = match self.last_quote_update {
                            Some(t) => format!("Prices updated {} · Yahoo, delayed ~15 min", t.format("%H:%M:%S")),
                            None => "Prices not updated yet · Yahoo, delayed ~15 min".to_owned(),
                        };
                        ui.label(RichText::new(text).size(12.5).color(MUTED)).on_hover_text(
                            "Current prices are refreshed from Yahoo Finance every 10 seconds. \
                             Yahoo's quotes for Vietnamese markets are delayed by about 15–20 \
                             minutes, so during trading hours this is the last traded price from \
                             roughly a quarter of an hour ago.",
                        );
                    });
                });
            });
    }

    fn table_card(&mut self, ui: &mut Ui, holdings: &[Holding]) {
        card_frame().show(ui, |ui| {
            ui.set_min_size(ui.available_size());

            if self.rows.is_empty() {
                empty_state(ui);
                return;
            }

            const IN_W: f32 = 92.0;
            let value_col = || Column::auto().at_least(96.0);
            let mut delete: Option<usize> = None;
            let focus_row = self.focus_row.take();

            // Sort state is shared with the header buttons through cells: the
            // header renders before the body, and a click takes effect next
            // frame. The stored copy is synced back after the table renders.
            let sort_key = Cell::new(self.sort_key);
            let sort_desc = Cell::new(self.sort_desc);
            let sort_clicked = Cell::new(false);
            // Display order: pinned rows first, then by the current option.
            let order = sort_order(&self.rows, holdings, self.sort_key, self.sort_desc);
            // The longest P/L % bar fills the whole column width.
            let max_pct = holdings.iter().filter_map(pl_pct).map(f64::abs).fold(0.0, f64::max);

            egui::ScrollArea::horizontal().show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .cell_layout(Layout::right_to_left(Align::Center))
                    .column(Column::exact(96.0)) // symbol
                    .column(Column::exact(IN_W)) // qty
                    .column(Column::exact(IN_W)) // cost price
                    .column(Column::exact(IN_W)) // current price
                    .column(value_col()) // total cost
                    .column(value_col()) // P/L
                    .column(Column::auto().at_least(70.0)) // P/L %
                    .column(Column::exact(IN_W)) // stop loss
                    .column(Column::auto().at_least(70.0)) // stop %
                    .column(value_col()) // total loss
                    .column(Column::exact(IN_W)) // target
                    .column(Column::auto().at_least(70.0)) // target %
                    .column(value_col()) // total gain
                    .column(Column::exact(30.0)) // delete
                    .header(34.0, |mut header| {
                        let heads: [(&str, Color32); 14] = [
                            ("SYMBOL", MUTED),
                            ("QUANTITY", MUTED),
                            ("COST PRICE", MUTED),
                            ("CURRENT", MUTED),
                            ("TOTAL COST", MUTED),
                            ("P/L", MUTED),
                            ("P/L %", MUTED),
                            ("STOP LOSS", RED),
                            ("STOP %", RED),
                            ("TOTAL LOSS", RED),
                            ("TARGET", GREEN),
                            ("TARGET %", GREEN),
                            ("TOTAL GAIN", GREEN),
                            ("", MUTED),
                        ];
                        for (i, (title, color)) in heads.into_iter().enumerate() {
                            // The four sortable columns.
                            let sort_col = match i {
                                0 => Some(SortKey::Symbol),
                                4 => Some(SortKey::TotalCost),
                                5 => Some(SortKey::Pl),
                                6 => Some(SortKey::PlPct),
                                _ => None,
                            };
                            header.col(|ui| {
                                let content = |ui: &mut Ui| {
                                    match sort_col {
                                        None => {
                                            ui.label(RichText::new(title).font(semibold(12.0)).color(color));
                                        }
                                        Some(key) => {
                                            let arrow = if sort_key.get() == key {
                                                if sort_desc.get() { " ▼" } else { " ▲" }
                                            } else {
                                                ""
                                            };
                                            let resp = ui.add(
                                                Button::new(
                                                    RichText::new(format!("{title}{arrow}"))
                                                        .font(semibold(12.0))
                                                        .color(color),
                                                )
                                                .frame(false),
                                            );
                                            if resp.clicked() {
                                                if sort_key.get() == key {
                                                    sort_desc.set(!sort_desc.get());
                                                } else {
                                                    sort_key.set(key);
                                                    // Numbers read best largest-first.
                                                    sort_desc.set(!matches!(key, SortKey::Symbol));
                                                }
                                                sort_clicked.set(true);
                                            }
                                        }
                                    }
                                };
                                if i == 0 {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add_space(4.0);
                                        content(ui);
                                    });
                                } else {
                                    content(ui);
                                }
                            });
                        }
                    })
                    .body(|mut body| {
                        for &i in &order {
                            let (row, h) = (&mut self.rows[i], &holdings[i]);
                            body.row(38.0, |mut r| {
                                r.col(|ui| {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        // Pin: filled when pinned, faint otherwise.
                                        let pin = Button::new(
                                            RichText::new("📌")
                                                .size(13.0)
                                                .color(if row.pinned {
                                                    ACCENT
                                                } else {
                                                    Color32::from_rgb(0xC7, 0xCE, 0xDB)
                                                }),
                                        )
                                        .frame(false);
                                        if ui
                                            .add(pin)
                                            .on_hover_text(if row.pinned {
                                                "Unpin (sorts normally)"
                                            } else {
                                                "Pin to top"
                                            })
                                            .clicked()
                                        {
                                            row.pinned = !row.pinned;
                                        }
                                        let resp = symbol_input_colored(
                                            ui,
                                            &mut row.symbol,
                                            64.0,
                                            Some(pl_color(h.unrealized())),
                                        );
                                        if focus_row == Some(i) {
                                            resp.request_focus();
                                        }
                                    });
                                });
                                r.col(|ui| {
                                    num_input(ui, &mut row.quantity, IN_W - 8.0, "0");
                                });
                                r.col(|ui| {
                                    num_input(ui, &mut row.cost_price, IN_W - 8.0, "0");
                                });
                                r.col(|ui| {
                                    num_input(ui, &mut row.current_price, IN_W - 8.0, "0");
                                });
                                r.col(|ui| {
                                    ui.label(RichText::new(fmt_int(h.total_cost())).color(TEXT));
                                });
                                r.col(|ui| {
                                    let pl = h.unrealized();
                                    let resp = ui.label(RichText::new(fmt_int(pl)).color(signed_color(pl)));
                                    if h.total_cost() > 0.0 && h.current_price > 0.0 {
                                        resp.on_hover_text(format!(
                                            "Market value {}\n{}",
                                            fmt_int(h.market_value()),
                                            fmt_signed_pct(pl / h.total_cost() * 100.0)
                                        ));
                                    }
                                });
                                r.col(|ui| {
                                    // Horizontal bar chart in the cell
                                    // background, starting at the left edge:
                                    // length proportional to the P/L % relative
                                    // to the table's biggest, red for a loss,
                                    // green for a gain.
                                    let pct = pl_pct(h);
                                    if let Some(p) = pct {
                                        if max_pct > 0.0 {
                                            let frac = ((p.abs() / max_pct) as f32).clamp(0.0, 1.0);
                                            let mut bar = ui.available_rect_before_wrap();
                                            bar.max.x = bar.min.x + bar.width() * frac;
                                            let c = if p > 0.0 { GREEN } else { RED };
                                            ui.painter().rect_filled(
                                                bar,
                                                2,
                                                Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 70),
                                            );
                                        }
                                    }
                                    match pct {
                                        Some(p) => {
                                            ui.label(
                                                RichText::new(fmt_signed_pct(p))
                                                    .font(semibold(12.5))
                                                    .color(signed_color(p)),
                                            );
                                        }
                                        None => {
                                            ui.label(RichText::new("–").color(MUTED));
                                        }
                                    };
                                });
                                r.col(|ui| {
                                    // Blink the cell's background as the price
                                    // closes in on the stop: light orange when
                                    // near, red and faster once below. The
                                    // value itself stays black.
                                    let bg = match stop_alert(h) {
                                        Some(true) => Some(blink_color(ui, FAST_BLINK, RED)),
                                        Some(false) => Some(blink_color(ui, SLOW_BLINK, ORANGE)),
                                        None => None,
                                    };
                                    num_input_bg(ui, &mut row.stop_loss, IN_W - 8.0, "0", bg);
                                });
                                r.col(|ui| pct_pill(ui, h.stop_pct()));
                                r.col(|ui| {
                                    let v = h.total_loss();
                                    ui.label(RichText::new(fmt_int(v)).color(signed_color(v)));
                                });
                                r.col(|ui| {
                                    num_input(ui, &mut row.target, IN_W - 8.0, "0");
                                });
                                r.col(|ui| pct_pill(ui, h.target_pct()));
                                r.col(|ui| {
                                    let v = h.total_gain();
                                    ui.label(RichText::new(fmt_int(v)).color(signed_color(v)));
                                });
                                r.col(|ui| {
                                    let btn = Button::new(RichText::new("✖").color(MUTED).size(13.0)).frame(false);
                                    if ui.add(btn).on_hover_text("Remove this stock").clicked() {
                                        delete = Some(i);
                                    }
                                });
                            });
                        }

                        // Totals row
                        let sum = |f: fn(&Holding) -> f64| holdings.iter().map(f).sum::<f64>();
                        body.row(40.0, |mut r| {
                            let strong = |v: f64, color: Color32| {
                                RichText::new(fmt_int(v)).font(semibold(15.0)).color(color)
                            };
                            r.col(|ui| {
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("TOTAL").font(semibold(13.0)).color(TEXT));
                                });
                            });
                            r.col(|_| {});
                            r.col(|_| {});
                            r.col(|_| {});
                            r.col(|ui| {
                                ui.label(strong(sum(Holding::total_cost), TEXT));
                            });
                            r.col(|ui| {
                                let v = sum(Holding::unrealized);
                                ui.label(strong(v, signed_color(v)));
                            });
                            r.col(|_| {});
                            r.col(|_| {});
                            r.col(|_| {});
                            r.col(|ui| {
                                let v = sum(Holding::total_loss);
                                ui.label(strong(v, signed_color(v)));
                            });
                            r.col(|_| {});
                            r.col(|_| {});
                            r.col(|ui| {
                                let v = sum(Holding::total_gain);
                                ui.label(strong(v, signed_color(v)));
                            });
                            r.col(|_| {});
                        });
                    });
            });

            // Sync header clicks back into the app state (the click happened
            // during rendering; the next frame displays the new order).
            if sort_clicked.get() {
                self.sort_key = sort_key.get();
                self.sort_desc = sort_desc.get();
            }

            if let Some(i) = delete {
                // Empty rows are removed without asking.
                if self.rows[i].to_holding() == Holding::default() {
                    self.rows.remove(i);
                } else {
                    self.confirm_delete = Some(i);
                }
            }
        });
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(i) = self.confirm_delete else { return };
        let Some(row) = self.rows.get(i) else {
            self.confirm_delete = None;
            return;
        };
        let name = if row.symbol.is_empty() { "this stock".to_owned() } else { row.symbol.clone() };

        let mut close = false;
        egui::Window::new("Remove stock")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(window_frame())
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Remove {name} from your portfolio?")).size(15.0));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let yes = Button::new(RichText::new("Remove").font(semibold(14.0)).color(Color32::WHITE))
                        .fill(RED)
                        .corner_radius(8);
                    if ui.add(yes).clicked() {
                        self.rows.remove(i);
                        close = true;
                    }
                    if ui.add(Button::new("Cancel").corner_radius(8)).clicked() {
                        close = true;
                    }
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.confirm_delete = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Small widgets

/// A small rounded badge showing a percentage, colored by sign.
fn pct_pill(ui: &mut Ui, pct: Option<f64>) {
    let Some(p) = pct else {
        ui.label(RichText::new("–").color(MUTED));
        return;
    };
    let (fg, bg) = if p >= 0.0 { (GREEN, GREEN_SOFT) } else { (RED, RED_SOFT) };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(20)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(fmt_signed_pct(p)).font(semibold(12.5)).color(fg));
        });
}

fn summary_cards(ui: &mut Ui, holdings: &[Holding]) {
    let sum = |f: fn(&Holding) -> f64| holdings.iter().map(f).sum::<f64>();
    let cost = sum(Holding::total_cost);
    let value: f64 = holdings
        .iter()
        .map(|h| if h.current_price > 0.0 { h.market_value() } else { h.total_cost() })
        .sum();
    let pl = sum(Holding::unrealized);
    let loss = sum(Holding::total_loss);
    let gain = sum(Holding::total_gain);

    let pl_sub = if cost > 0.0 { fmt_signed_pct(pl / cost * 100.0) } else { String::new() };
    let rr_sub = if loss < 0.0 && gain > 0.0 {
        format!("Reward : risk  {} : 1", fmt_int(gain / -loss))
    } else {
        String::new()
    };

    let cards: [(&str, &str, String, Color32, String); 6] = [
        (
            // No pictograph: the installed fonts don't cover one for this card.
            "",
            "Symbols",
            holdings.iter().filter(|h| !h.symbol.is_empty()).count().to_string(),
            TEXT,
            String::new(),
        ),
        ("💼", "Total cost", fmt_int(cost), TEXT, String::new()),
        ("🏦", "Market value", fmt_int(value), TEXT, String::new()),
        ("📊", "Unrealized P/L", fmt_int(pl), signed_color(pl), pl_sub),
        ("🛡", "Loss at stops", fmt_int(loss), signed_color(loss), String::new()),
        ("🎯", "Gain at targets", fmt_int(gain), signed_color(gain), rr_sub),
    ];

    let gap = 14.0;
    let n = cards.len() as f32;
    let w = ((ui.available_width() - gap * (n - 1.0)) / n).max(150.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (icon, title, value, color, sub) in cards {
            card_frame().show(ui, |ui| {
                ui.set_width(w - 28.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        if !icon.is_empty() {
                            ui.label(RichText::new(icon).size(14.0));
                        }
                        ui.label(RichText::new(title).size(13.5).color(MUTED));
                    });
                    ui.label(RichText::new(value).font(semibold(22.0)).color(color));
                    // Always reserve the subtitle line so all cards share one height.
                    let sub_color = if color == TEXT { MUTED } else { color };
                    ui.label(RichText::new(if sub.is_empty() { " " } else { &sub }).size(12.5).color(sub_color));
                });
            });
        }
    });
}

fn empty_state(ui: &mut Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.28);
        ui.label(RichText::new("🌱").size(44.0));
        ui.add_space(6.0);
        ui.label(RichText::new("Your portfolio is empty").font(semibold(20.0)).color(TEXT));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Click “Add stock” to add a position, or use the Stock Calculator to size a new trade.")
                .size(14.5)
                .color(MUTED),
        );
    });
}

/// Adds imported positions to the table, merging by symbol: quantities add
/// up and the cost price becomes the weighted average of the old and
/// imported lots (1'000 at 10'000 plus 1'000 at 12'000 gives 2'000 at
/// 11'000). A row that already exists keeps its stop loss and target; its
/// market price is only refreshed when the file has one. A symbol repeated
/// inside one file is the same position listed twice, so the later entry
/// wins before merging.
fn merge_rows(rows: &mut Vec<RowUi>, holdings: &[Holding]) {
    // Collapse the batch: last entry per symbol wins.
    let mut batch: Vec<&Holding> = Vec::new();
    for h in holdings {
        match batch.iter_mut().find(|b| b.symbol == h.symbol) {
            Some(b) => *b = h,
            None => batch.push(h),
        }
    }
    for h in batch {
        match rows.iter_mut().find(|r| r.symbol == h.symbol) {
            Some(row) => {
                let old_qty = parse_or_zero(&row.quantity);
                let old_cost = parse_or_zero(&row.cost_price);
                let total_qty = old_qty + h.quantity;
                // Adding zero shares must not shift the average.
                let avg_cost = if total_qty > 0.0 {
                    (old_qty * old_cost + h.quantity * h.cost_price) / total_qty
                } else {
                    h.cost_price
                };
                row.quantity = fmt_input(total_qty);
                row.cost_price = fmt_input(avg_cost);
                if h.current_price > 0.0 {
                    row.current_price = fmt_input(h.current_price);
                }
            }
            None => rows.push(RowUi::from_holding(h)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(stop: f64, current: f64) -> Holding {
        Holding { stop_loss: stop, current_price: current, ..Default::default() }
    }

    #[test]
    fn merge_rows_averages_cost_and_appends_new() {
        // The classic case: 1'000 at 10'000 plus 1'000 at 12'000 gives
        // 2'000 at 11'000.
        let mut rows = vec![RowUi::from_holding(&Holding {
            symbol: "ABC".into(),
            quantity: 1_000.0,
            cost_price: 10_000.0,
            current_price: 9_500.0,
            stop_loss: 9_000.0,
            target: 12_000.0,
            ..Default::default()
        })];
        merge_rows(
            &mut rows,
            &[Holding { symbol: "ABC".into(), quantity: 1_000.0, cost_price: 12_000.0, ..Default::default() }],
        );

        assert_eq!(parse_or_zero(&rows[0].quantity), 2_000.0);
        assert_eq!(parse_or_zero(&rows[0].cost_price), 11_000.0);
        // The file had no market price, so the saved one stays.
        assert_eq!(parse_or_zero(&rows[0].current_price), 9_500.0);
        // Stop loss and target are the user's plan, not broker data.
        assert_eq!(parse_or_zero(&rows[0].stop_loss), 9_000.0);
        assert_eq!(parse_or_zero(&rows[0].target), 12_000.0);

        // Uneven lots average to a whole number; stop/target still kept.
        let mut rows = vec![RowUi::from_holding(&Holding {
            symbol: "TCB".into(),
            quantity: 1_000.0,
            cost_price: 35_000.0,
            current_price: 32_500.0,
            stop_loss: 33_000.0,
            target: 40_000.0,
            ..Default::default()
        })];
        merge_rows(
            &mut rows,
            &[
                Holding { symbol: "TCB".into(), quantity: 2_000.0, cost_price: 34_000.0, current_price: 32_450.0, ..Default::default() },
                // New symbols, the second one twice: appended once, last wins.
                Holding { symbol: "FPT".into(), quantity: 2_000.0, cost_price: 69_000.0, current_price: 63_400.0, ..Default::default() },
                Holding { symbol: "FPT".into(), quantity: 2_500.0, cost_price: 69_500.0, current_price: 63_500.0, ..Default::default() },
            ],
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(parse_or_zero(&rows[0].quantity), 3_000.0);
        assert_eq!(parse_or_zero(&rows[0].cost_price), 34_333.0);
        assert_eq!(parse_or_zero(&rows[0].stop_loss), 33_000.0);
        assert_eq!(parse_or_zero(&rows[0].target), 40_000.0);
        assert_eq!(parse_or_zero(&rows[0].current_price), 32_450.0);
        assert_eq!(rows[1].symbol, "FPT");
        assert_eq!(parse_or_zero(&rows[1].quantity), 2_500.0);
    }

    #[test]
    fn sort_order_pins_first_then_sorts() {
        let mk = |symbol: &str, qty: f64, cost: f64, current: f64, pinned: bool| {
            RowUi::from_holding(&Holding {
                symbol: symbol.into(),
                quantity: qty,
                cost_price: cost,
                current_price: current,
                pinned,
                ..Default::default()
            })
        };
        let rows = vec![
            mk("TCB", 1_000.0, 35_000.0, 32_500.0, false),  // cost 35M, P/L -2.5M, -7.1%
            mk("FPT", 2_000.0, 69_000.0, 63_400.0, true),   // cost 138M, P/L -11.2M, -8.1%
            mk("VPB", 13_333.0, 23_500.0, 23_000.0, false), // cost 313M, P/L -6.7M, -2.1%
            RowUi::default(),                               // empty row: always last
        ];
        let holdings: Vec<Holding> = rows.iter().map(RowUi::to_holding).collect();

        // Symbol, ascending: pinned FPT first, then alphabetical, empty last.
        assert_eq!(sort_order(&rows, &holdings, SortKey::Symbol, false), vec![1, 0, 2, 3]);
        // Total cost descending: pinned FPT (138M) first, then VPB (313M), TCB (35M).
        assert_eq!(sort_order(&rows, &holdings, SortKey::TotalCost, true), vec![1, 2, 0, 3]);
        // Total cost ascending: pinned FPT first, then TCB, VPB.
        assert_eq!(sort_order(&rows, &holdings, SortKey::TotalCost, false), vec![1, 0, 2, 3]);
        // P/L descending (biggest loss first): pinned FPT, then TCB, VPB.
        assert_eq!(sort_order(&rows, &holdings, SortKey::Pl, true), vec![1, 0, 2, 3]);
        // P/L % ascending: pinned FPT (-8.1%) first, then TCB (-7.1%), VPB (-2.1%).
        assert_eq!(sort_order(&rows, &holdings, SortKey::PlPct, false), vec![1, 0, 2, 3]);
        // P/L % descending: pinned FPT first, then VPB (-2.1%), TCB (-7.1%).
        assert_eq!(sort_order(&rows, &holdings, SortKey::PlPct, true), vec![1, 2, 0, 3]);
    }

    #[test]
    fn stop_alert_thresholds() {
        // No stop or no current price: no alert.
        assert_eq!(stop_alert(&holding(0.0, 100.0)), None);
        assert_eq!(stop_alert(&holding(95.0, 0.0)), None);
        // Comfortably above the stop.
        assert_eq!(stop_alert(&holding(95.0, 100.0)), None);
        // Exactly 0.5% above the stop: still a slow blink.
        assert_eq!(stop_alert(&holding(100.0, 100.5)), Some(false));
        // Within 0.5% above the stop: slow blink.
        assert_eq!(stop_alert(&holding(100.0, 100.2)), Some(false));
        // Below the stop: rapid blink.
        assert_eq!(stop_alert(&holding(100.0, 99.0)), Some(true));
        assert_eq!(stop_alert(&holding(100.0, 50.0)), Some(true));
    }

    #[test]
    fn symbol_color_follows_pl() {
        assert_eq!(pl_color(-1.0), RED);
        assert_eq!(pl_color(1.0), GREEN);
        assert_eq!(pl_color(0.0), TEXT);
    }
}
