//! stockcalc.exe — standalone position-size calculator.

#![windows_subsystem = "windows"] // no console window on Windows

use eframe::egui::{self, Button, Color32, Margin, RichText, Ui, Vec2};
use stock_calc::format::{fmt_input, fmt_money, fmt_num, fmt_signed_pct, num_input, parse_or_zero, symbol_input};
use stock_calc::model::{self, CalculatorData, Holding};
use stock_calc::theme::*;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Stock Calculator")
            .with_inner_size([400.0, 590.0])
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
        Self {
            symbol: data.symbol.clone(),
            entry: fmt_input(data.entry),
            stop_loss: fmt_input(data.stop_loss),
            risk_amount: fmt_input(data.risk_amount),
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
        }
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

        self.autosave();
    }
}

impl CalcApp {
    fn body(&mut self, ui: &mut Ui) {
        const W: f32 = 180.0;
        egui::Grid::new("calc_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            field_label(ui, "Symbol");
            symbol_input(ui, &mut self.symbol, W);
            ui.end_row();

            field_label(ui, "Entry price");
            num_input(ui, &mut self.entry, W, true, "0.00");
            ui.end_row();

            field_label(ui, "Stop loss price");
            num_input(ui, &mut self.stop_loss, W, true, "0.00");
            ui.end_row();

            field_label(ui, "Risk amount");
            num_input(ui, &mut self.risk_amount, W, true, "0.00");
            ui.end_row();
        });

        ui.add_space(14.0);

        let entry = parse_or_zero(&self.entry);
        let stop = parse_or_zero(&self.stop_loss);
        let risk = parse_or_zero(&self.risk_amount);
        let per_share = entry - stop;

        let error = if entry <= 0.0 || risk <= 0.0 || stop <= 0.0 {
            Some("Fill in entry, stop loss and risk amount.")
        } else if stop >= entry {
            Some("Stop loss must be below the entry price.")
        } else {
            None
        };
        let qty = if error.is_none() { (risk / per_share).floor() } else { 0.0 };

        egui::Frame::new()
            .fill(if error.is_none() { ACCENT_SOFT } else { INPUT_BG })
            .corner_radius(12)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("QUANTITY TO BUY").font(semibold(12.0)).color(MUTED));
                    let qty_text = if error.is_none() { fmt_num(qty, 0) } else { "–".to_owned() };
                    ui.label(RichText::new(qty_text).font(semibold(34.0)).color(ACCENT));
                });
                ui.add_space(6.0);

                match error {
                    Some(msg) => {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(msg).size(13.0).color(MUTED));
                        });
                    }
                    None => {
                        egui::Grid::new("calc_result").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                            result_line(ui, "Risk per share", fmt_money(per_share), TEXT);
                            result_line(ui, "Stop distance", fmt_signed_pct(-per_share / entry * 100.0), RED);
                            result_line(ui, "Position size", fmt_money(qty * entry), TEXT);
                            result_line(ui, "Actual risk", fmt_money(qty * per_share), RED);
                        });
                    }
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let can_add = error.is_none() && qty > 0.0;
            let add = Button::new(RichText::new("➕  Add to portfolio").font(semibold(14.0)).color(Color32::WHITE))
                .fill(ACCENT)
                .corner_radius(8)
                .min_size(Vec2::new(0.0, 32.0));
            if ui
                .add_enabled(can_add, add)
                .on_hover_text("Adds this position to the portfolio with the entry as cost price")
                .clicked()
            {
                let h = Holding {
                    symbol: self.symbol.clone(),
                    quantity: qty,
                    cost_price: entry,
                    current_price: entry,
                    stop_loss: stop,
                    target: 0.0,
                };
                self.message = match model::append_holding(h) {
                    Ok(()) => {
                        let name = if self.symbol.is_empty() { "Position" } else { &self.symbol };
                        Message::Info(format!("✔ {name} ({} shares) added to portfolio", fmt_num(qty, 0)))
                    }
                    Err(e) => Message::Error(e),
                };
            }
            let clear = Button::new("Clear").corner_radius(8).min_size(Vec2::new(0.0, 32.0));
            if ui.add(clear).clicked() {
                self.symbol.clear();
                self.entry.clear();
                self.stop_loss.clear();
                // The risk amount is usually the same each trade, so keep it.
                self.message = Message::None;
            }
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
}
