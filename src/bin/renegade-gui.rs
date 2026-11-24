//! Graphical front end: the same features as the `renegade` CLI, built on `renegade::mouse`.
//!
//! All device I/O runs on a worker thread (the mouse may sleep and take seconds to answer);
//! the UI edits a `Model` copy and "Применить" writes only the sections that differ from what was read.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{env, fs, thread};

use eframe::egui::{
    self, Color32, ComboBox, CornerRadius, DragValue, FontData, FontDefinitions, FontFamily,
    FontId, Frame, Margin, RichText, Slider, Stroke, TextEdit, TextStyle, Theme, ThemePreference,
};
use renegade::config::{LEVELS, PHYSICAL_SLOTS, SLOTS, Sensor, parse_binding};
use renegade::device::Device;
use renegade::macros::{self, Repeat};
use renegade::mouse::{self, Button, Led, Res, State};

const BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x18);
const SIDEBAR: Color32 = Color32::from_rgb(0x17, 0x18, 0x1e);
const CARD: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x26);
const FIELD: Color32 = Color32::from_rgb(0x28, 0x2b, 0x34);
const FIELD_HOVER: Color32 = Color32::from_rgb(0x33, 0x37, 0x42);
const BORDER: Color32 = Color32::from_rgb(0x2e, 0x31, 0x3b);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe9, 0xee);
const MUTED: Color32 = Color32::from_rgb(0x8d, 0x92, 0xa0);
const ACCENT: Color32 = Color32::from_rgb(0xff, 0x6b, 0x3d);
const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0x30, 0x14, 0x0b, 0x30);
const ON_ACCENT: Color32 = Color32::from_rgb(0x1a, 0x10, 0x0c);
const OK: Color32 = Color32::from_rgb(0x4c, 0xcf, 0x8a);
const ERR: Color32 = Color32::from_rgb(0xff, 0x5d, 0x5d);

/// Button actions offered directly in the list, as (`parse_binding` name, label).
const SIMPLE: [(&str, &str); 11] = [
    ("left", "Левый клик"),
    ("right", "Правый клик"),
    ("middle", "Средняя кнопка"),
    ("back", "Назад"),
    ("forward", "Вперёд"),
    ("dpi-up", "DPI +"),
    ("dpi-down", "DPI −"),
    ("dpi-cycle", "DPI по кругу"),
    ("double-click", "Двойной клик"),
    ("triple-click", "Тройной клик"),
    ("off", "Отключена"),
];
const OFF: usize = 10;
const LOGO_LABELS: [&str; 3] = ["Выкл", "Статичный", "Дыхание"];
const DPI_LED_LABELS: [&str; 4] = ["Выкл", "Статичный", "Дыхание", "Неон"];
const POLL_RATES: [u32; 4] = [125, 250, 500, 1000];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Renegade Wireless")
            .with_app_id("renegade-gui")
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Renegade Wireless",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

// ---------- worker thread ----------

type Job = Box<dyn FnOnce(&mut Device) -> Res<Option<State>> + Send>;
type Outcome = Result<Option<State>, String>;

/// Runs device jobs one by one off the UI thread; a job may return fresh `State` to show.
struct Worker {
    jobs: Sender<(String, Job)>,
    done: Receiver<(String, Outcome)>,
}

impl Worker {
    fn spawn(ctx: egui::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<(String, Job)>();
        let (done_tx, done) = mpsc::channel();
        thread::spawn(move || {
            let mut dev: Option<Device> = None;
            for (label, job) in job_rx {
                let outcome = run_job(&mut dev, job).map_err(|e| e.to_string());
                // Reopen next time: the receiver may have been replugged.
                if outcome.is_err() {
                    dev = None;
                }
                if done_tx.send((label, outcome)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self { jobs, done }
    }
}

fn run_job(dev: &mut Option<Device>, job: Job) -> Res<Option<State>> {
    let d = match dev {
        Some(d) => d,
        None => dev.insert(Device::open()?),
    };
    job(d)
}

// ---------- editable model ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Simple(usize),
    Click,
    Key,
    Macro,
    Raw,
}

/// One button's editor state; fields not used by `kind` are kept so switching back restores them.
#[derive(Clone, PartialEq)]
struct ButtonEdit {
    kind: Kind,
    interval: u8,
    count: u8,
    key: String,
    raw: String,
    repeat: Repeat,
    events: String,
}

impl ButtonEdit {
    fn from_button(b: &Button) -> Self {
        let mut e = Self {
            kind: Kind::Simple(OFF),
            interval: 50,
            count: 2,
            key: String::new(),
            raw: String::new(),
            repeat: Repeat::Times(1),
            events: String::new(),
        };
        match b {
            Button::Macro { repeat, events } => {
                e.kind = Kind::Macro;
                e.repeat = *repeat;
                e.events = events.clone().unwrap_or_default();
            }
            Button::Action(a) => {
                if let Some(i) = SIMPLE.iter().position(|(n, _)| n == a) {
                    e.kind = Kind::Simple(i);
                } else if let Some((i, c)) =
                    a.strip_prefix("click:").and_then(|r| r.split_once(':'))
                {
                    e.kind = Kind::Click;
                    e.interval = i.parse().unwrap_or(50);
                    e.count = c.parse().unwrap_or(2);
                } else if let Some(k) = a.strip_prefix("key:") {
                    e.kind = Kind::Key;
                    e.key = k.into();
                } else {
                    e.kind = Kind::Raw;
                    e.raw = a.strip_prefix("raw:").unwrap_or(a).into();
                }
            }
        }
        e
    }

    /// `parse_binding` text for every kind except macros.
    fn action(&self) -> String {
        match self.kind {
            Kind::Simple(i) => SIMPLE[i].0.into(),
            Kind::Click => format!("click:{}:{}", self.interval, self.count),
            Kind::Key => format!("key:{}", self.key.trim()),
            Kind::Raw => format!("raw:{}", self.raw.trim()),
            Kind::Macro => String::new(),
        }
    }

    fn tokens(&self) -> Vec<&str> {
        self.events.split_whitespace().collect()
    }

    fn check(&self) -> Result<(), String> {
        match self.kind {
            Kind::Macro => macros::parse(&self.tokens()).map(|_| ()),
            _ => parse_binding(&self.action()).map(|_| ()),
        }
    }

    fn write(&self, dev: &mut Device, slot: usize) -> Res<()> {
        match self.kind {
            Kind::Macro => mouse::set_macro(dev, slot, self.repeat, &self.tokens()),
            _ => mouse::set_button(dev, slot, &self.action()),
        }
    }
}

/// Everything the UI edits; compared against the last read copy to find what to write.
#[derive(Clone, PartialEq)]
struct Model {
    sensor: Sensor,
    poll: u32,
    dpi: Vec<u32>,
    colors: [[u8; 3]; LEVELS],
    logo: Led,
    dpi_led: Led,
    dpi_led_color: [u8; 3],
    buttons: Vec<ButtonEdit>,
}

impl Model {
    fn from_state(s: &State) -> Self {
        let (dpi_led, dpi_led_color) = s.dpi_led();
        Self {
            sensor: s.sensor,
            poll: s.poll_hz().unwrap_or(1000),
            dpi: (0..s.dpi_count().max(1))
                .map(|i| s.dpi(i).map_or(800, |(x, _)| x))
                .collect(),
            colors: std::array::from_fn(|i| s.color(i)),
            logo: s.logo(),
            dpi_led,
            dpi_led_color,
            buttons: s.buttons.iter().map(ButtonEdit::from_button).collect(),
        }
    }

    fn has_errors(&self) -> bool {
        self.buttons.iter().any(|b| b.check().is_err())
    }

    /// Writes the sections that differ from `base`, then rereads the mouse.
    fn write_changes(&self, base: &Self, dev: &mut Device) -> Res<Option<State>> {
        if self.poll != base.poll {
            mouse::set_polling(dev, self.poll)?;
        }
        if self.dpi != base.dpi {
            mouse::set_dpi(dev, self.sensor, &self.dpi)?;
        }
        for i in (0..LEVELS).filter(|&i| self.colors[i] != base.colors[i]) {
            mouse::set_dpi_color(dev, i, self.colors[i])?;
        }
        if self.logo != base.logo {
            mouse::set_logo(dev, self.logo)?;
        }
        if (self.dpi_led, self.dpi_led_color) != (base.dpi_led, base.dpi_led_color) {
            mouse::set_dpi_led(dev, self.dpi_led, self.dpi_led_color)?;
        }
        // New left-click bindings first, so the "keep one left click" guard never trips halfway.
        let mut changed: Vec<usize> = (0..SLOTS)
            .filter(|&i| self.buttons[i] != base.buttons[i])
            .collect();
        changed.sort_by_key(|&i| self.buttons[i].kind != Kind::Simple(0));
        for i in changed {
            self.buttons[i].write(dev, i)?;
        }
        Ok(Some(State::read(dev)?))
    }
}

/// DPI values the sensor accepts, ascending.
fn valid_dpis(sensor: Sensor) -> Vec<u32> {
    (100..=16000)
        .step_by(100)
        .filter(|d| sensor.encode(*d).is_some())
        .collect()
}

// ---------- app ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dpi,
    Light,
    Buttons,
    Backup,
}

struct App {
    worker: Worker,
    busy: Option<String>,
    status: Option<(bool, String)>,
    base: Option<Model>,
    edit: Option<Model>,
    info: String,
    tab: Tab,
    backup_path: String,
    confirm_restore: bool,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        setup_style(ctx);
        let home = env::var("HOME").unwrap_or_else(|_| ".".into());
        let mut app = Self {
            worker: Worker::spawn(ctx.clone()),
            busy: None,
            status: None,
            base: None,
            edit: None,
            info: String::new(),
            tab: Tab::Dpi,
            backup_path: format!("{home}/renegade-backup.bin"),
            confirm_restore: false,
        };
        app.reload();
        app
    }

    fn submit(&mut self, label: &str, job: Job) {
        if self.busy.is_none() && self.worker.jobs.send((label.into(), job)).is_ok() {
            self.busy = Some(label.into());
        }
    }

    fn reload(&mut self) {
        self.submit("Чтение настроек", Box::new(|d| Ok(Some(State::read(d)?))));
    }

    fn collect_results(&mut self) {
        while let Ok((label, outcome)) = self.worker.done.try_recv() {
            self.busy = None;
            self.status = Some(match outcome {
                Ok(state) => {
                    if let Some(s) = state {
                        self.info = format!("{:?} · bcdDevice {:#06x}", s.sensor, s.version);
                        self.base = Some(Model::from_state(&s));
                        self.edit = self.base.clone();
                    }
                    (true, format!("{label}: готово"))
                }
                Err(e) => (false, format!("{label}: {e}")),
            });
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Renegade Wireless")
                        .font(semibold(22.0))
                        .color(TEXT),
                );
                let sub = if self.info.is_empty() {
                    "ARDOR GAMING".to_string()
                } else {
                    format!("ARDOR GAMING · {}", self.info)
                };
                ui.label(RichText::new(sub).color(MUTED).small());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.busy.is_none(), egui::Button::new("Перечитать с мыши"))
                    .clicked()
                {
                    self.reload();
                }
            });
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(label) = &self.busy {
                ui.spinner();
                ui.label(
                    RichText::new(format!("{label}… если долго нет ответа, подвигайте мышь"))
                        .color(MUTED),
                );
            } else if let Some((ok, msg)) = &self.status {
                ui.label(RichText::new(msg).color(if *ok { OK } else { ERR }));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (Some(base), Some(edit)) = (self.base.clone(), self.edit.clone()) else {
                    return;
                };
                let dirty = base != edit;
                let can_apply = dirty && self.busy.is_none() && !edit.has_errors();
                let apply = egui::Button::new(RichText::new("Применить").color(ON_ACCENT).strong())
                    .fill(ACCENT)
                    .min_size(egui::vec2(120.0, 34.0));
                if ui.add_enabled(can_apply, apply).clicked() {
                    let (base, edit) = (base.clone(), edit.clone());
                    self.submit(
                        "Запись настроек",
                        Box::new(move |d| edit.write_changes(&base, d)),
                    );
                    return;
                }
                if dirty
                    && ui
                        .add(
                            egui::Button::new("Отменить изменения").min_size(egui::vec2(0.0, 34.0)),
                        )
                        .clicked()
                {
                    self.edit = self.base.clone();
                }
                if dirty && edit.has_errors() {
                    ui.label(RichText::new("исправьте ошибки в кнопках").color(ERR));
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        for (tab, label) in [
            (Tab::Dpi, "DPI и опрос"),
            (Tab::Light, "Подсветка"),
            (Tab::Buttons, "Кнопки и макросы"),
            (Tab::Backup, "Резервная копия"),
        ] {
            let b = egui::Button::selectable(self.tab == tab, RichText::new(label).size(15.0))
                .min_size(egui::vec2(ui.available_width(), 38.0));
            if ui.add(b).clicked() {
                self.tab = tab;
            }
            ui.add_space(4.0);
        }
    }

    fn page(&mut self, ui: &mut egui::Ui) {
        if self.tab == Tab::Backup {
            self.backup_page(ui);
            return;
        }
        let Some(m) = &mut self.edit else {
            let text = if self.busy.is_some() {
                "Чтение настроек мыши…"
            } else {
                "Мышь не прочитана — нажмите «Перечитать с мыши»."
            };
            ui.label(RichText::new(text).color(MUTED));
            return;
        };
        match self.tab {
            Tab::Dpi => dpi_page(ui, m),
            Tab::Light => light_page(ui, m),
            Tab::Buttons => buttons_page(ui, m),
            Tab::Backup => {}
        }
    }

    fn backup_page(&mut self, ui: &mut egui::Ui) {
        card(
            ui,
            "Резервная копия памяти мыши",
            |ui| {
                muted(
                    ui,
                    "Все настройки, клавиши и макросы. Восстановление перезаписывает только отличающиеся байты.",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Файл");
                    ui.add(
                        TextEdit::singleline(&mut self.backup_path).desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(8.0);
                let idle = self.busy.is_none();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(idle, egui::Button::new("Сохранить в файл"))
                        .clicked()
                    {
                        let path = self.backup_path.clone();
                        self.submit(
                            "Резервная копия",
                            Box::new(move |d| {
                                fs::write(&path, mouse::backup(d)?)?;
                                Ok(None)
                            }),
                        );
                    }
                    if !self.confirm_restore {
                        if ui
                            .add_enabled(idle, egui::Button::new("Восстановить из файла…"))
                            .clicked()
                        {
                            self.confirm_restore = true;
                        }
                    } else {
                        ui.label(
                            RichText::new("Перезаписать настройки мыши из файла?").color(ACCENT),
                        );
                        if ui
                            .add_enabled(idle, egui::Button::new("Да, восстановить"))
                            .clicked()
                        {
                            self.confirm_restore = false;
                            let path = self.backup_path.clone();
                            self.submit(
                                "Восстановление",
                                Box::new(move |d| {
                                    mouse::restore(d, &fs::read(&path)?)?;
                                    Ok(Some(State::read(d)?))
                                }),
                            );
                        }
                        if ui.button("Отмена").clicked() {
                            self.confirm_restore = false;
                        }
                    }
                });
            },
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.collect_results();
        let bar = |fill| {
            Frame::new()
                .fill(fill)
                .inner_margin(Margin::symmetric(20, 12))
        };
        egui::Panel::top("header")
            .frame(bar(SIDEBAR))
            .show(ui, |ui| self.header(ui));
        egui::Panel::bottom("footer")
            .frame(bar(SIDEBAR))
            .show(ui, |ui| self.footer(ui));
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(210.0)
            .frame(
                Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::same(20)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| self.page(ui));
            });
    }
}

// ---------- pages ----------

fn dpi_page(ui: &mut egui::Ui, m: &mut Model) {
    card(ui, "Частота опроса", |ui| {
        ui.horizontal(|ui| {
            for hz in POLL_RATES {
                ui.selectable_value(&mut m.poll, hz, format!("{hz} Гц"));
            }
        });
    });
    card(ui, "Уровни DPI", |ui| {
        muted(
            ui,
            &format!(
                "Кнопка DPI переключает уровни по кругу. Сенсор {:?}: {}.",
                m.sensor,
                m.sensor.range()
            ),
        );
        ui.add_space(8.0);
        let valid = valid_dpis(m.sensor);
        let mut remove = None;
        for i in 0..m.dpi.len() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}", i + 1))
                        .font(semibold(16.0))
                        .color(MUTED),
                );
                ui.color_edit_button_srgb(&mut m.colors[i]);
                let mut idx = valid
                    .iter()
                    .position(|v| *v >= m.dpi[i])
                    .unwrap_or(valid.len() - 1);
                ui.spacing_mut().slider_width = 340.0;
                ui.add(Slider::new(&mut idx, 0..=valid.len() - 1).show_value(false));
                m.dpi[i] = valid[idx];
                ui.label(
                    RichText::new(format!("{} DPI", m.dpi[i]))
                        .font(semibold(16.0))
                        .color(TEXT),
                );
                if m.dpi.len() > 1 && ui.button("Удалить").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            m.dpi.remove(i);
            // Keep each color attached to its level.
            m.colors[i..].rotate_left(1);
        }
        if m.dpi.len() < LEVELS && ui.button("+ Добавить уровень").clicked() {
            let last = m.dpi.last().copied().unwrap_or(800);
            m.dpi.push(last);
        }
    });
}

fn light_page(ui: &mut egui::Ui, m: &mut Model) {
    card(ui, "Логотип", |ui| {
        led_editor(ui, "logo", &LOGO_LABELS, &mut m.logo, None)
    });
    card(ui, "DPI-индикатор", |ui| {
        led_editor(
            ui,
            "dpi-led",
            &DPI_LED_LABELS,
            &mut m.dpi_led,
            Some(&mut m.dpi_led_color),
        );
    });
    muted(
        ui,
        "Названия режимов восстановлены по Windows-программе; проверьте результат на мыши.",
    );
}

fn led_editor(
    ui: &mut egui::Ui,
    id: &str,
    labels: &[&str],
    led: &mut Led,
    color: Option<&mut [u8; 3]>,
) {
    ui.horizontal(|ui| {
        for (i, label) in labels.iter().enumerate() {
            ui.selectable_value(&mut led.mode, i as u8, *label);
        }
    });
    if led.mode == 0 {
        return;
    }
    ui.add_space(6.0);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            if let Some(c) = color {
                ui.label("Цвет");
                ui.color_edit_button_srgb(c);
                ui.end_row();
            }
            ui.label("Яркость");
            ui.add(Slider::new(&mut led.brightness, 1..=10));
            ui.end_row();
            // Speed only matters for the animated modes (OemDrv writes it only for those).
            if led.mode >= 2 {
                ui.label("Скорость");
                ui.add(Slider::new(&mut led.speed, 1..=10));
                ui.end_row();
            }
        });
}

/// Factory function of each physical slot (Cfg.ini), as a hint.
fn factory(slot1: usize) -> &'static str {
    match slot1 {
        1 => "левый клик",
        2 => "правый клик",
        3 => "средняя",
        4 => "назад",
        5 => "вперёд",
        9 => "огонь",
        11 => "DPI +",
        12 => "DPI −",
        _ => "отключена",
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Simple(i) => SIMPLE[i].1,
        Kind::Click => "Серия кликов",
        Kind::Key => "Клавиша / сочетание",
        Kind::Macro => "Макрос",
        Kind::Raw => "Raw-код",
    }
}

fn buttons_page(ui: &mut egui::Ui, m: &mut Model) {
    muted(ui, "Хотя бы одна кнопка должна остаться левым кликом.");
    ui.add_space(8.0);
    for slot in 0..SLOTS {
        let shown =
            PHYSICAL_SLOTS.contains(&(slot + 1)) || m.buttons[slot].kind != Kind::Simple(OFF);
        if shown {
            card(ui, &format!("Кнопка {}", slot + 1), |ui| {
                button_editor(ui, slot, &mut m.buttons[slot])
            });
        }
    }
}

fn button_editor(ui: &mut egui::Ui, slot: usize, e: &mut ButtonEdit) {
    if PHYSICAL_SLOTS.contains(&(slot + 1)) {
        muted(ui, &format!("заводская функция: {}", factory(slot + 1)));
    }
    ui.horizontal(|ui| {
        ComboBox::from_id_salt(("kind", slot))
            .width(220.0)
            .selected_text(kind_label(e.kind))
            .show_ui(ui, |ui| {
                for (i, (_, label)) in SIMPLE.iter().enumerate() {
                    ui.selectable_value(&mut e.kind, Kind::Simple(i), *label);
                }
                ui.separator();
                for k in [Kind::Click, Kind::Key, Kind::Macro, Kind::Raw] {
                    ui.selectable_value(&mut e.kind, k, kind_label(k));
                }
            });
        match e.kind {
            Kind::Click => {
                ui.label("интервал");
                ui.add(DragValue::new(&mut e.interval).range(1..=255));
                ui.label("кликов");
                ui.add(DragValue::new(&mut e.count).range(1..=255));
            }
            Kind::Key => {
                ui.add(
                    TextEdit::singleline(&mut e.key)
                        .hint_text("ctrl+shift+a")
                        .desired_width(220.0),
                );
            }
            Kind::Raw => {
                ui.add(
                    TextEdit::singleline(&mut e.raw)
                        .hint_text("070000")
                        .desired_width(120.0),
                );
            }
            Kind::Simple(_) | Kind::Macro => {}
        }
    });
    if e.kind == Kind::Macro {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Повтор:");
            let times = match e.repeat {
                Repeat::Times(n) => n,
                _ => 1,
            };
            if ui
                .selectable_label(matches!(e.repeat, Repeat::Times(_)), "N раз")
                .clicked()
            {
                e.repeat = Repeat::Times(times);
            }
            if let Repeat::Times(n) = &mut e.repeat {
                ui.add(DragValue::new(n).range(1..=253));
            }
            ui.selectable_value(&mut e.repeat, Repeat::Hold, "пока кнопка нажата");
            ui.selectable_value(&mut e.repeat, Repeat::UntilPress, "до нажатия любой кнопки");
        });
        ui.add(
            TextEdit::multiline(&mut e.events)
                .hint_text("ctrl+c 50ms ctrl+v")
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .font(TextStyle::Monospace),
        );
        muted(
            ui,
            "a · ctrl+c · down:shift / up:shift · left right middle back forward · 50ms — пауза после предыдущего",
        );
    }
    if let Err(msg) = e.check() {
        ui.label(RichText::new(msg).color(ERR));
    }
}

// ---------- look ----------

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(semibold(16.0)).color(TEXT));
            ui.add_space(8.0);
            body(ui);
        });
    ui.add_space(12.0);
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).small());
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn setup_style(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    fonts.font_data.insert(
        "geologica".into(),
        font(include_bytes!("../../assets/fonts/Geologica-Regular.ttf")),
    );
    fonts.font_data.insert(
        "geologica-semibold".into(),
        font(include_bytes!("../../assets/fonts/Geologica-SemiBold.ttf")),
    );
    let mut fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fallback.insert(0, "geologica".into());
    fonts
        .families
        .insert(FontFamily::Proportional, fallback.clone());
    fallback.insert(0, "geologica-semibold".into());
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), fallback);
    ctx.set_fonts(fonts);

    ctx.set_theme(ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(22.0, FontFamily::Name("semibold".into())),
            ),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.5)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = CARD;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT_SOFT;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.slider_trailing_fill = true;
        let radius = CornerRadius::same(8);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, CARD),
            (&mut v.widgets.inactive, FIELD),
            (&mut v.widgets.hovered, FIELD_HOVER),
            (&mut v.widgets.active, FIELD_HOVER),
            (&mut v.widgets.open, FIELD_HOVER),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = radius;
            w.bg_stroke = Stroke::NONE;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    });
}
