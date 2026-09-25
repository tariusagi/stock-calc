//! portfolio.exe — the stock portfolio manager.

#![windows_subsystem = "windows"] // no console window on Windows

use std::time::{Duration, SystemTime};

use eframe::egui::{self, Align, Button, Color32, Layout, Margin, RichText, Stroke, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use stock_calc::format::{fmt_input, fmt_int, fmt_signed_pct, num_input, parse_or_zero, symbol_input};
use stock_calc::instance::{self, App};
use stock_calc::model::{self, Holding, PortfolioFile};
use stock_calc::theme::*;

fn main() -> eframe::Result {
    if !instance::claim(App::Portfolio) {
        instance::hand_over(App::Portfolio);
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(App::Portfolio.title())
            .with_inner_size([1400.0, 740.0])
            .with_min_inner_size([900.0, 480.0]),
        ..Default::default()
    };
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
        }
    }
}

enum Status {
    Saved,
    Error(String),
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
}

impl PortfolioApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let mut app = Self {
            rows: Vec::new(),
            last_saved: PortfolioFile::default(),
            known_mtime: None,
            status: Status::Saved,
            confirm_delete: None,
            focus_row: None,
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
}

impl eframe::App for PortfolioApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Pick up positions added by the Stock Calculator.
        if model::portfolio_modified() != self.known_mtime {
            self.reload();
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));

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
                ui.horizontal(|ui| match &self.status {
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
                    .column(Column::exact(IN_W)) // stop loss
                    .column(Column::auto().at_least(70.0)) // stop %
                    .column(value_col()) // total loss
                    .column(Column::exact(IN_W)) // target
                    .column(Column::auto().at_least(70.0)) // target %
                    .column(value_col()) // total gain
                    .column(Column::exact(30.0)) // delete
                    .header(34.0, |mut header| {
                        let heads: [(&str, Color32); 13] = [
                            ("SYMBOL", MUTED),
                            ("QUANTITY", MUTED),
                            ("COST PRICE", MUTED),
                            ("CURRENT", MUTED),
                            ("TOTAL COST", MUTED),
                            ("P/L", MUTED),
                            ("STOP LOSS", RED),
                            ("STOP %", RED),
                            ("TOTAL LOSS", RED),
                            ("TARGET", GREEN),
                            ("TARGET %", GREEN),
                            ("TOTAL GAIN", GREEN),
                            ("", MUTED),
                        ];
                        for (i, (title, color)) in heads.into_iter().enumerate() {
                            header.col(|ui| {
                                let text = RichText::new(title).font(semibold(12.0)).color(color);
                                if i == 0 {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add_space(4.0);
                                        ui.label(text);
                                    });
                                } else {
                                    ui.label(text);
                                }
                            });
                        }
                    })
                    .body(|mut body| {
                        for (i, (row, h)) in self.rows.iter_mut().zip(holdings).enumerate() {
                            body.row(38.0, |mut r| {
                                r.col(|ui| {
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        let resp = symbol_input(ui, &mut row.symbol, 84.0);
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
                                    num_input(ui, &mut row.stop_loss, IN_W - 8.0, "0");
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

    let cards: [(&str, &str, String, Color32, String); 5] = [
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
                        ui.label(RichText::new(icon).size(14.0));
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
