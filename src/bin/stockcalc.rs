//! stockcalc.exe — standalone position-size calculator with a per-symbol history.

#![windows_subsystem = "windows"] // no console window on Windows

use eframe::egui::{
    self, Align, Button, Color32, CursorIcon, Layout, Margin, RichText, Sense, Stroke, Ui, UiBuilder, Vec2,
    ViewportCommand,
};
use stock_calc::format::{fmt_input, fmt_int, fmt_signed_pct, num_input, parse_or_zero, symbol_input};
use stock_calc::instance::{self, App};
use stock_calc::model::{self, CalculatorData, HistoryEntry, Holding, Sizing};
use stock_calc::theme::*;

const CALC_WIDTH: f32 = 400.0;
const HISTORY_WIDTH: f32 = 340.0;
const HEIGHT: f32 = 636.0;
const HISTORY_ICON: &str = "↺";

fn main() -> eframe::Result {
    if !instance::claim(App::Calculator) {
        instance::hand_over(App::Calculator);
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(App::Calculator.title())
            .with_inner_size([CALC_WIDTH, HEIGHT])
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    eframe::run_native("Stock Calculator", options, Box::new(|cc| Ok(Box::new(CalcApp::new(cc)))))
}

enum Message {
    None,
    Info(String),
    Error(String),
}

struct CalcApp {
    symbol: String,
    entry: String,
    stop_loss: String,
    risk_amount: String,
    history: Vec<HistoryEntry>,
    show_history: bool,
    /// Inputs were edited and not yet recorded in the history. Recording
    /// waits until the symbol field loses focus, so a half-typed symbol
    /// ("T", "TS", ...) never creates its own entry.
    pending_record: bool,
    symbol_focused: bool,
    last_saved: CalculatorData,
    message: Message,
}

impl CalcApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let (data, message) = match model::load_calculator() {
            Ok(d) => (d, Message::None),
            Err(e) => (CalculatorData::default(), Message::Error(e)),
        };
        let mut history = data.history.clone();
        history.sort_by(|a, b| b.modified.cmp(&a.modified));
        Self {
            symbol: data.symbol.clone(),
            entry: fmt_input(data.entry),
            stop_loss: fmt_input(data.stop_loss),
            risk_amount: fmt_input(data.risk_amount),
            history,
            show_history: false,
            pending_record: false,
            symbol_focused: false,
            last_saved: data,
            message,
        }
    }

    fn data(&self) -> CalculatorData {
        CalculatorData {
            symbol: self.symbol.clone(),
            entry: parse_or_zero(&self.entry),
            stop_loss: parse_or_zero(&self.stop_loss),
            risk_amount: parse_or_zero(&self.risk_amount),
            history: self.history.clone(),
        }
    }

    fn sizing(&self) -> Result<Sizing, &'static str> {
        Sizing::compute(
            parse_or_zero(&self.entry),
            parse_or_zero(&self.stop_loss),
            parse_or_zero(&self.risk_amount),
        )
    }

    fn record_history(&mut self) {
        if !self.pending_record || self.symbol_focused || self.symbol.is_empty() || self.sizing().is_err() {
            return;
        }
        let mut data = self.data();
        data.record_history();
        self.history = data.history;
        self.pending_record = false;
    }

    fn load_entry(&mut self, e: &HistoryEntry) {
        self.symbol = e.symbol.clone();
        self.entry = fmt_input(e.entry);
        self.stop_loss = fmt_input(e.stop_loss);
        self.risk_amount = fmt_input(e.risk_amount);
        self.pending_record = false;
        self.message = Message::None;
    }

    fn toggle_history(&mut self, ctx: &egui::Context) {
        self.show_history = !self.show_history;
        let width = if self.show_history { CALC_WIDTH + HISTORY_WIDTH } else { CALC_WIDTH };
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(width, HEIGHT)));
    }

    fn autosave(&mut self) {
        let data = self.data();
        if data != self.last_saved {
            if let Err(e) = model::save_calculator(&data) {
                self.message = Message::Error(format!("Could not save: {e}"));
            }
            self.last_saved = data;
        }
    }
}

impl eframe::App for CalcApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Added first so it spans the full window height.
        if self.show_history {
            self.history_panel(ui);
        }

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(ACCENT).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🖩").size(26.0).color(Color32::WHITE));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("Stock Calculator").font(semibold(21.0)).color(Color32::WHITE));
                        ui.label(RichText::new("Size a position by the money you risk").size(13.0).color(ACCENT_MUTED));
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(16)))
            .show(ui, |ui| {
                card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.body(ui);
                });
            });

        self.record_history();
        self.autosave();
    }
}

// ---------------------------------------------------------------------------
// Calculator

impl CalcApp {
    fn body(&mut self, ui: &mut Ui) {
        const W: f32 = 180.0;
        let mut edited = false;
        egui::Grid::new("calc_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            field_label(ui, "Symbol");
            let resp = symbol_input(ui, &mut self.symbol, W);
            edited |= resp.changed();
            self.symbol_focused = resp.has_focus();
            ui.end_row();

            field_label(ui, "Entry price");
            edited |= num_input(ui, &mut self.entry, W, "0");
            ui.end_row();

            field_label(ui, "Stop loss price");
            edited |= num_input(ui, &mut self.stop_loss, W, "0");
            ui.end_row();

            field_label(ui, "Risk amount");
            edited |= num_input(ui, &mut self.risk_amount, W, "0");
            ui.end_row();
        });
        self.pending_record |= edited;

        ui.add_space(14.0);

        let sizing = self.sizing();
        egui::Frame::new()
            .fill(if sizing.is_ok() { ACCENT_SOFT } else { INPUT_BG })
            .corner_radius(12)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("QUANTITY TO BUY").font(semibold(12.0)).color(MUTED));
                    let qty_text = sizing.map(|s| fmt_int(s.quantity)).unwrap_or_else(|_| "–".to_owned());
                    ui.label(RichText::new(qty_text).font(semibold(34.0)).color(ACCENT));
                });
                ui.add_space(6.0);

                match sizing {
                    Err(msg) => {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(msg).size(13.0).color(MUTED));
                        });
                    }
                    Ok(s) => {
                        egui::Grid::new("calc_result").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                            result_line(ui, "Risk per share", fmt_int(s.risk_per_share), TEXT);
                            result_line(ui, "Stop distance", fmt_signed_pct(s.stop_pct), RED);
                            result_line(ui, "Position size", fmt_int(s.position_size), TEXT);
                            result_line(ui, "Actual risk", fmt_int(s.actual_risk), RED);
                        });
                    }
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let can_add = sizing.is_ok_and(|s| s.quantity > 0.0);
            let add = Button::new(RichText::new("➕  Add to portfolio").font(semibold(14.0)).color(Color32::WHITE))
                .fill(ACCENT)
                .corner_radius(8)
                .min_size(Vec2::new(0.0, 32.0));
            if ui
                .add_enabled(can_add, add)
                .on_hover_text("Adds this position to the portfolio with the entry as cost price")
                .clicked()
                && let Ok(s) = sizing
            {
                self.add_to_portfolio(s);
            }
            let portfolio = Button::new(RichText::new("📈 Portfolio").font(semibold(14.0)).color(ACCENT))
                .fill(ACCENT_SOFT)
                .stroke(Stroke::new(1.0, ACCENT_MUTED))
                .corner_radius(8)
                .min_size(Vec2::new(0.0, 32.0));
            if ui.add(portfolio).on_hover_text("Open the portfolio").clicked()
                && let Err(e) = instance::open(App::Portfolio)
            {
                self.message = Message::Error(e);
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let clear = Button::new("Clear").corner_radius(8).min_size(Vec2::new(0.0, 32.0));
            if ui.add(clear).clicked() {
                self.symbol.clear();
                self.entry.clear();
                self.stop_loss.clear();
                // The risk amount is usually the same each trade, so keep it.
                self.message = Message::None;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (fill, stroke, color) = if self.show_history {
                    (ACCENT_SOFT, Stroke::new(1.5, ACCENT), ACCENT)
                } else {
                    (INPUT_BG, Stroke::new(1.0, BORDER), TEXT)
                };
                let btn = Button::new(RichText::new(HISTORY_ICON).size(19.0).color(color))
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(8)
                    .min_size(Vec2::new(36.0, 32.0));
                let tip = if self.show_history { "Hide history" } else { "Show history" };
                if ui.add(btn).on_hover_text(tip).clicked() {
                    self.toggle_history(ui.ctx());
                }
            });
        });

        match &self.message {
            Message::None => {}
            Message::Info(m) => {
                ui.add_space(6.0);
                ui.label(RichText::new(m).size(13.0).color(GREEN));
            }
            Message::Error(m) => {
                ui.add_space(6.0);
                ui.label(RichText::new(m).size(13.0).color(RED));
            }
        }
    }

    fn add_to_portfolio(&mut self, s: Sizing) {
        let entry = parse_or_zero(&self.entry);
        let h = Holding {
            symbol: self.symbol.clone(),
            quantity: s.quantity,
            cost_price: entry,
            current_price: entry,
            stop_loss: parse_or_zero(&self.stop_loss),
            target: 0.0,
        };
        self.message = match model::append_holding(h) {
            Ok(()) => {
                // Show the new position right away if the portfolio is open.
                instance::activate(App::Portfolio);
                let name = if self.symbol.is_empty() { "Position" } else { &self.symbol };
                Message::Info(format!("✔ {name} ({} shares) added to portfolio", fmt_int(s.quantity)))
            }
            Err(e) => Message::Error(e),
        };
    }
}

// ---------------------------------------------------------------------------
// History panel

impl CalcApp {
    fn history_panel(&mut self, ui: &mut Ui) {
        let mut close = false;
        let mut load: Option<usize> = None;
        let mut remove: Option<usize> = None;

        egui::Panel::right("history")
            .exact_size(HISTORY_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(Margin::symmetric(14, 14)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(HISTORY_ICON).size(18.0).color(ACCENT));
                    ui.label(RichText::new("History").font(semibold(18.0)).color(TEXT));
                    ui.label(RichText::new(format!("{}", self.history.len())).size(13.0).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let x = Button::new(RichText::new("✖").size(14.0).color(MUTED)).frame(false);
                        if ui.add(x).on_hover_text("Close history").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if self.history.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No calculations yet").font(semibold(15.0)).color(TEXT));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Every symbol you calculate is saved here.").size(13.0).color(MUTED),
                        );
                    });
                    return;
                }

                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for (i, e) in self.history.iter().enumerate() {
                        let current = e.symbol == self.symbol;
                        match history_card(ui, e, current) {
                            CardAction::Load => load = Some(i),
                            CardAction::Remove => remove = Some(i),
                            CardAction::None => {}
                        }
                        ui.add_space(8.0);
                    }
                });
            });

        if let Some(i) = load {
            let e = self.history[i].clone();
            self.load_entry(&e);
        }
        if let Some(i) = remove {
            self.history.remove(i);
        }
        if close {
            self.toggle_history(ui.ctx());
        }
    }
}

enum CardAction {
    None,
    Load,
    Remove,
}

fn history_card(ui: &mut Ui, e: &HistoryEntry, current: bool) -> CardAction {
    let mut action = CardAction::None;

    // The clickable area is created before its contents, so the "x" button
    // inside stays on top and gets its own clicks.
    let resp = ui
        .scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            let hovered = ui.response().hovered();
            let fill = if hovered { ACCENT_SOFT } else { INPUT_BG };
            let stroke = if current { Stroke::new(1.5, ACCENT) } else { Stroke::new(1.0, BORDER) };

            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(10)
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&e.symbol).font(semibold(16.0)).color(TEXT));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let x = Button::new(RichText::new("✖").size(11.0).color(MUTED)).frame(false);
                            if ui.add(x).on_hover_text("Remove from history").clicked() {
                                action = CardAction::Remove;
                            }
                            ui.label(RichText::new(fmt_timestamp(e.modified)).size(12.0).color(MUTED));
                        });
                    });
                    ui.add_space(2.0);

                    let sizing = e.sizing();
                    egui::Grid::new(("hist", &e.symbol))
                        .num_columns(4)
                        .spacing([8.0, 3.0])
                        .show(ui, |ui| {
                            let pair = |ui: &mut Ui, label: &str, value: String, color: Color32| {
                                ui.label(RichText::new(label).size(12.5).color(MUTED));
                                ui.label(RichText::new(value).font(semibold(13.0)).color(color));
                            };
                            pair(ui, "Entry", fmt_int(e.entry), TEXT);
                            pair(ui, "Stop", fmt_int(e.stop_loss), RED);
                            ui.end_row();
                            pair(ui, "Risk", fmt_int(e.risk_amount), TEXT);
                            match sizing {
                                Ok(s) => {
                                    pair(ui, "Stop %", fmt_signed_pct(s.stop_pct), RED);
                                    ui.end_row();
                                    pair(ui, "Quantity", fmt_int(s.quantity), ACCENT);
                                    pair(ui, "Risk/share", fmt_int(s.risk_per_share), TEXT);
                                    ui.end_row();
                                    pair(ui, "Position", fmt_int(s.position_size), TEXT);
                                    pair(ui, "Actual risk", fmt_int(s.actual_risk), RED);
                                    ui.end_row();
                                }
                                Err(_) => ui.end_row(),
                            }
                        });
                });
        })
        .response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Load into the calculator");

    if resp.clicked() && matches!(action, CardAction::None) {
        action = CardAction::Load;
    }
    action
}

fn fmt_timestamp(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%d %b %Y  %H:%M").to_string())
        .unwrap_or_default()
}
