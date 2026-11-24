//! «Макросы» tab: a library of named macros on disk, an event table editor and a recorder.
//!
//! Compared to OemDrv: delays are edited in place, rows move with buttons, recording can use real
//! timing or a fixed pause, mouse clicks are recorded in a capture pad, and any event can be added
//! by typing tokens (`ctrl+c 50ms left`). Library files are the same token text the CLI reads with `@file`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Rect, RichText, Sense, TextEdit};
use renegade::config::{BUTTON_FACTORY, BUTTON_SLOTS};
use renegade::macros::{self, Input, MAX_EVENTS, NAME_MAX, Repeat};

use crate::{ACCENT, BORDER, ERR, FIELD, MUTED, OK, TEXT, card, muted, repeat_editor, semibold};

/// Delay OemDrv never goes below.
const MIN_DELAY: u16 = 3;

/// A macro being edited.
#[derive(Clone, Default, PartialEq)]
struct Doc {
    name: String,
    events: Vec<macros::Event>,
    /// Library file it came from, if any.
    path: Option<PathBuf>,
}

/// Request to bind the edited macro to a button (applied to the model by the app).
pub struct Assign {
    pub slot: usize,
    pub repeat: Repeat,
    pub name: String,
    pub events: String,
}

struct Recorder {
    last: Instant,
    mods: Modifiers,
}

pub struct MacroEditor {
    library: Vec<PathBuf>,
    filter: String,
    doc: Doc,
    saved: Doc,
    /// Selected row: new events are inserted before it.
    selected: Option<usize>,
    recording: Option<Recorder>,
    /// Row whose key is being replaced by the next key press.
    rebind: Option<usize>,
    auto_delay: bool,
    fixed_delay: u16,
    all_delays: u16,
    add_text: String,
    assign_button: usize,
    assign_repeat: Repeat,
    message: Option<(bool, String)>,
    confirm_delete: bool,
    pad: Rect,
}

impl MacroEditor {
    pub fn new() -> Self {
        let mut e = Self {
            library: Vec::new(),
            filter: String::new(),
            doc: Doc::default(),
            saved: Doc::default(),
            selected: None,
            recording: None,
            rebind: None,
            auto_delay: true,
            fixed_delay: 20,
            all_delays: 20,
            add_text: String::new(),
            assign_button: 3,
            assign_repeat: Repeat::Times(1),
            message: None,
            confirm_delete: false,
            pad: Rect::NOTHING,
        };
        e.refresh();
        e
    }

    /// Library file names (without extension), for pickers elsewhere.
    pub fn names(&self) -> Vec<(String, PathBuf)> {
        self.library.iter().map(|p| (stem(p), p.clone())).collect()
    }

    pub fn refresh(&mut self) {
        let mut files: Vec<PathBuf> = fs::read_dir(macros::library_dir())
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|x| x == "txt"));
        files.sort_by_key(|p| stem(p).to_lowercase());
        self.library = files;
    }

    /// Opens a macro (e.g. one read from a button) as an unsaved document.
    pub fn open(&mut self, name: &str, tokens: &str) {
        let events = parse_text(tokens).unwrap_or_default();
        let path = self.library.iter().find(|p| stem(p) == name).cloned();
        self.doc = Doc {
            name: name.into(),
            events,
            path,
        };
        self.saved = Doc::default();
        self.selected = None;
        self.message = None;
    }

    fn load(&mut self, path: &Path) {
        match macros::load_file(path) {
            Ok((name, text)) => match parse_text(&text) {
                Ok(events) => {
                    self.doc = Doc {
                        name,
                        events,
                        path: Some(path.to_path_buf()),
                    };
                    self.saved = self.doc.clone();
                    self.selected = None;
                    self.message = None;
                }
                Err(e) => self.message = Some((false, format!("{}: {e}", stem(path)))),
            },
            Err(e) => self.message = Some((false, e)),
        }
    }

    fn save(&mut self) {
        let name = self.doc.name.trim().to_string();
        if let Err(e) = check_name(&name) {
            self.message = Some((false, e));
            return;
        }
        let dir = macros::library_dir();
        let path = dir.join(format!("{name}.txt"));
        let text = macros::describe(&self.doc.events);
        let result = fs::create_dir_all(&dir).and_then(|()| fs::write(&path, format!("{text}\n")));
        if let Err(e) = result {
            self.message = Some((false, format!("{}: {e}", path.display())));
            return;
        }
        // Renamed: drop the old file.
        if let Some(old) = self.doc.path.take().filter(|old| *old != path) {
            let _ = fs::remove_file(old);
        }
        self.doc.name = name;
        self.doc.path = Some(path);
        self.saved = self.doc.clone();
        self.refresh();
        self.message = Some((true, format!("сохранено в {}", dir.display())));
    }

    fn delete(&mut self) {
        if let Some(path) = &self.doc.path {
            let _ = fs::remove_file(path);
        }
        self.doc = Doc::default();
        self.saved = Doc::default();
        self.refresh();
    }

    fn dirty(&self) -> bool {
        self.doc != self.saved
    }

    /// Takes keyboard and pad clicks while recording or rebinding; call before any widget runs.
    pub fn capture(&mut self, ui: &mut egui::Ui) {
        if self.recording.is_none() && self.rebind.is_none() {
            return;
        }
        let events = ui.input_mut(|i| {
            let taken = i.events.clone();
            i.events.retain(|e| {
                !matches!(
                    e,
                    Event::Key { .. } | Event::Text(_) | Event::ModifiersChanged(_)
                )
            });
            taken
        });
        for e in events {
            match e {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat: false,
                    ..
                } => {
                    // Physical key: the layout (e.g. Russian) must not change what is recorded.
                    let Some(input) = key_input(physical_key.unwrap_or(key)) else {
                        continue;
                    };
                    if let Some(row) = self.rebind.take() {
                        self.rebind_row(row, input);
                    } else {
                        self.record(pressed, input);
                    }
                }
                Event::ModifiersChanged(m) if self.recording.is_some() => self.record_mods(m),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if self.recording.is_some() && self.pad.contains(pos) => {
                    if let Some(mask) = mouse_mask(button) {
                        self.record(pressed, Input::Mouse(mask));
                    }
                }
                _ => {}
            }
        }
    }

    fn record_mods(&mut self, now: Modifiers) {
        let Some(rec) = &self.recording else { return };
        let before = rec.mods;
        for (bit, was, is) in [
            (1, before.ctrl, now.ctrl),
            (2, before.shift, now.shift),
            (4, before.alt, now.alt),
        ] {
            if was != is {
                self.record(is, Input::Mod(bit));
            }
        }
        if let Some(rec) = &mut self.recording {
            rec.mods = now;
        }
    }

    fn record(&mut self, down: bool, input: Input) {
        let Some(rec) = &mut self.recording else {
            return;
        };
        if self.doc.events.len() >= MAX_EVENTS {
            self.recording = None;
            self.message = Some((
                false,
                format!("достигнут предел {MAX_EVENTS} событий, запись остановлена"),
            ));
            return;
        }
        let now = Instant::now();
        let at = self.selected.unwrap_or(self.doc.events.len());
        // The pause belongs to the event before the new one.
        if let Some(prev) = at.checked_sub(1).and_then(|i| self.doc.events.get_mut(i)) {
            prev.delay = if self.auto_delay {
                u16::try_from(now.duration_since(rec.last).as_millis())
                    .unwrap_or(u16::MAX)
                    .max(MIN_DELAY)
            } else {
                self.fixed_delay.max(MIN_DELAY)
            };
        }
        rec.last = now;
        // Its own pause is unknown until the next event arrives.
        let delay = if self.auto_delay {
            MIN_DELAY
        } else {
            self.fixed_delay.max(MIN_DELAY)
        };
        self.doc
            .events
            .insert(at, macros::Event { down, input, delay });
        if let Some(s) = &mut self.selected {
            *s += 1;
        }
    }

    /// Replaces the key of a row and of its matching press/release.
    fn rebind_row(&mut self, row: usize, input: Input) {
        let Some(old) = self.doc.events.get(row).copied() else {
            return;
        };
        let pair = if old.down {
            (row + 1..self.doc.events.len())
                .find(|&i| !self.doc.events[i].down && self.doc.events[i].input == old.input)
        } else {
            (0..row)
                .rev()
                .find(|&i| self.doc.events[i].down && self.doc.events[i].input == old.input)
        };
        for i in std::iter::once(row).chain(pair) {
            self.doc.events[i].input = input;
        }
    }

    fn insert_tokens(&mut self) {
        match parse_text(&self.add_text) {
            Ok(new) if self.doc.events.len() + new.len() <= MAX_EVENTS => {
                let at = self.selected.unwrap_or(self.doc.events.len());
                let n = new.len();
                self.doc.events.splice(at..at, new);
                if let Some(s) = &mut self.selected {
                    *s += n;
                }
                self.add_text.clear();
                self.message = None;
            }
            Ok(_) => {
                self.message = Some((false, format!("не больше {MAX_EVENTS} событий в макросе")))
            }
            Err(e) => self.message = Some((false, e)),
        }
    }

    /// Draws the tab; returns a request to bind the macro to a button.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Assign> {
        let mut assign = None;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(220.0);
                self.library_ui(ui);
            });
            ui.vertical(|ui| {
                self.header_ui(ui);
                self.recorder_ui(ui);
                self.events_ui(ui);
                assign = self.assign_ui(ui);
            });
        });
        assign
    }

    fn library_ui(&mut self, ui: &mut egui::Ui) {
        card(ui, "Библиотека", |ui| {
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text("поиск")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);
            let filter = self.filter.to_lowercase();
            let mut open = None;
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .id_salt("library")
                .show(ui, |ui| {
                    for path in self
                        .library
                        .iter()
                        .filter(|p| stem(p).to_lowercase().contains(&filter))
                    {
                        let current = self.doc.path.as_ref() == Some(path);
                        let r = ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::selectable(current, stem(path)),
                        );
                        if r.clicked() {
                            open = Some(path.clone());
                        }
                    }
                    if self.library.is_empty() {
                        muted(ui, "пока пусто — запишите макрос и сохраните");
                    }
                });
            if let Some(p) = open {
                self.load(&p);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("+ Новый").clicked() {
                    self.doc = Doc::default();
                    self.saved = Doc::default();
                    self.selected = None;
                }
                if self.doc.path.is_some() {
                    if !self.confirm_delete {
                        if ui.button("Удалить").clicked() {
                            self.confirm_delete = true;
                        }
                    } else if ui
                        .button(RichText::new("Точно удалить?").color(ERR))
                        .clicked()
                    {
                        self.confirm_delete = false;
                        self.delete();
                    }
                }
            });
            muted(ui, &macros::library_dir().display().to_string());
        });
    }

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        card(ui, "Макрос", |ui| {
            ui.horizontal(|ui| {
                ui.label("Имя");
                ui.add(
                    TextEdit::singleline(&mut self.doc.name)
                        .char_limit(NAME_MAX)
                        .hint_text("до 15 символов")
                        .desired_width(220.0),
                );
                let save = egui::Button::new(if self.dirty() {
                    "Сохранить •"
                } else {
                    "Сохранить"
                });
                if ui.add_enabled(!self.doc.events.is_empty(), save).clicked() {
                    self.save();
                }
                let total: u32 = self.doc.events.iter().map(|e| u32::from(e.delay)).sum();
                muted(
                    ui,
                    &format!(
                        "{} / {MAX_EVENTS} событий · ≈ {total} мс",
                        self.doc.events.len()
                    ),
                );
            });
            if let Some((ok, msg)) = &self.message {
                ui.label(RichText::new(msg).color(if *ok { OK } else { ERR }));
            }
        });
    }

    fn recorder_ui(&mut self, ui: &mut egui::Ui) {
        card(ui, "Запись", |ui| {
            ui.horizontal(|ui| {
                let label = if self.recording.is_some() {
                    "Остановить запись"
                } else {
                    "Начать запись"
                };
                let fill = if self.recording.is_some() {
                    ERR
                } else {
                    ACCENT
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).color(crate::ON_ACCENT).strong())
                            .fill(fill),
                    )
                    .clicked()
                {
                    self.recording = match self.recording {
                        Some(_) => None,
                        None => Some(Recorder {
                            last: Instant::now(),
                            mods: ui.input(|i| i.modifiers),
                        }),
                    };
                    self.rebind = None;
                }
                ui.checkbox(&mut self.auto_delay, "реальные паузы");
                ui.add_enabled_ui(!self.auto_delay, |ui| {
                    ui.label("иначе");
                    ui.add(
                        egui::DragValue::new(&mut self.fixed_delay)
                            .range(MIN_DELAY..=10000)
                            .suffix(" мс"),
                    );
                });
            });
            // Capture pad: mouse clicks inside it are recorded (the buttons above are not).
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), Sense::click());
            self.pad = rect;
            let active = self.recording.is_some();
            let stroke = egui::Stroke::new(1.0, if active { ACCENT } else { BORDER });
            ui.painter()
                .rect(rect, 8.0, FIELD, stroke, egui::StrokeKind::Inside);
            let text = if active {
                "Идёт запись: нажимайте клавиши, а кнопки мыши — над этой областью"
            } else {
                "Клавиши и клики над этой областью записываются после «Начать запись»"
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(14.0),
                if active { TEXT } else { MUTED },
            );
            if self.selected.is_some() {
                muted(
                    ui,
                    "новые события вставляются перед выбранной строкой (щёлкните № ещё раз, чтобы снять выбор)",
                );
            }
        });
    }

    fn events_ui(&mut self, ui: &mut egui::Ui) {
        card(ui, "События", |ui| {
            let mut action: Option<(usize, RowAction)> = None;
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .id_salt("events")
                .show(ui, |ui| {
                    egui::Grid::new("events-grid")
                        .striped(true)
                        .num_columns(6)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            for (i, e) in self.doc.events.iter_mut().enumerate() {
                                let selected = self.selected == Some(i);
                                if ui
                                    .selectable_label(selected, format!("{:>2}", i + 1))
                                    .clicked()
                                {
                                    action = Some((i, RowAction::Select));
                                }
                                let dir = if e.down {
                                    "↓ нажать"
                                } else {
                                    "↑ отпустить"
                                };
                                if ui
                                    .add(egui::Button::new(dir).min_size(egui::vec2(110.0, 0.0)))
                                    .on_hover_text("переключить")
                                    .clicked()
                                {
                                    e.down = !e.down;
                                }
                                let waiting = self.rebind == Some(i);
                                let key = if waiting {
                                    "нажмите клавишу…".to_string()
                                } else {
                                    macros::input_name(e.input)
                                };
                                let key_btn =
                                    egui::Button::new(RichText::new(key).font(semibold(14.0)))
                                        .min_size(egui::vec2(150.0, 0.0));
                                if ui
                                    .add(key_btn)
                                    .on_hover_text("щёлкните и нажмите другую клавишу")
                                    .clicked()
                                {
                                    action = Some((i, RowAction::Rebind));
                                }
                                ui.add(
                                    egui::DragValue::new(&mut e.delay)
                                        .range(MIN_DELAY..=u16::MAX)
                                        .suffix(" мс"),
                                );
                                ui.horizontal(|ui| {
                                    if ui.small_button("▲").clicked() {
                                        action = Some((i, RowAction::Up));
                                    }
                                    if ui.small_button("▼").clicked() {
                                        action = Some((i, RowAction::Down));
                                    }
                                    if ui.small_button("✕").clicked() {
                                        action = Some((i, RowAction::Delete));
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    if self.doc.events.is_empty() {
                        muted(
                            ui,
                            "событий нет — нажмите «Начать запись» или добавьте ниже",
                        );
                    }
                });
            if let Some((i, a)) = action {
                self.apply_row(i, a);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let r = ui.add(
                    TextEdit::singleline(&mut self.add_text)
                        .hint_text("добавить: ctrl+c 50ms left")
                        .desired_width(260.0),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.button("Добавить").clicked() || enter {
                    self.insert_tokens();
                }
                ui.separator();
                ui.add(
                    egui::DragValue::new(&mut self.all_delays)
                        .range(MIN_DELAY..=10000)
                        .suffix(" мс"),
                );
                if ui.button("всем паузам").clicked() {
                    self.doc
                        .events
                        .iter_mut()
                        .for_each(|e| e.delay = self.all_delays);
                }
                if ui.button("Очистить").clicked() {
                    self.doc.events.clear();
                    self.selected = None;
                }
            });
            muted(
                ui,
                "a · f5 · ctrl+c · down:shift / up:shift · left right middle back forward · 50ms — пауза после предыдущего",
            );
        });
    }

    fn apply_row(&mut self, i: usize, a: RowAction) {
        let ev = &mut self.doc.events;
        match a {
            RowAction::Select => {
                self.selected = if self.selected == Some(i) {
                    None
                } else {
                    Some(i)
                }
            }
            RowAction::Rebind => {
                self.recording = None;
                self.rebind = Some(i);
            }
            RowAction::Up if i > 0 => ev.swap(i, i - 1),
            RowAction::Down if i + 1 < ev.len() => ev.swap(i, i + 1),
            RowAction::Delete => {
                ev.remove(i);
                self.selected = self.selected.filter(|&s| s < ev.len());
            }
            RowAction::Up | RowAction::Down => {}
        }
    }

    fn assign_ui(&mut self, ui: &mut egui::Ui) -> Option<Assign> {
        let mut out = None;
        card(ui, "Назначить на кнопку", |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("assign-button")
                    .width(240.0)
                    .selected_text(button_label(self.assign_button))
                    .show_ui(ui, |ui| {
                        for b in 0..BUTTON_SLOTS.len() {
                            ui.selectable_value(&mut self.assign_button, b, button_label(b));
                        }
                    });
                let ok = !self.doc.events.is_empty();
                if ui.add_enabled(ok, egui::Button::new("Назначить")).clicked() {
                    out = Some(Assign {
                        slot: BUTTON_SLOTS[self.assign_button],
                        repeat: self.assign_repeat,
                        name: self.doc.name.trim().to_string(),
                        events: macros::describe(&self.doc.events),
                    });
                }
            });
            repeat_editor(ui, "assign-repeat", &mut self.assign_repeat);
            muted(
                ui,
                "Назначение попадает в настройки кнопки; в мышь всё запишет «Применить».",
            );
        });
        out
    }
}

#[derive(Clone, Copy)]
enum RowAction {
    Select,
    Rebind,
    Up,
    Down,
    Delete,
}

/// "N — заводская: …" label of a physical button (0-based).
pub fn button_label(b: usize) -> String {
    format!(
        "Кнопка {} — заводская: {}",
        b + 1,
        factory_label(BUTTON_FACTORY[b])
    )
}

fn factory_label(action: &str) -> &str {
    match action {
        "left" => "левый клик",
        "right" => "правый клик",
        "middle" => "колесо",
        "forward" => "вперёд",
        "back" => "назад",
        "dpi-up" => "DPI +",
        "dpi-down" => "DPI −",
        _ => "огонь",
    }
}

fn parse_text(text: &str) -> Result<Vec<macros::Event>, String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    macros::parse(&tokens)
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("задайте имя макроса".into());
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        || name.starts_with('.')
    {
        return Err("имя содержит недопустимые символы".into());
    }
    Ok(())
}

fn stem(p: &std::path::Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn mouse_mask(b: PointerButton) -> Option<u8> {
    match b {
        PointerButton::Primary => Some(0x01),
        PointerButton::Secondary => Some(0x02),
        PointerButton::Middle => Some(0x04),
        PointerButton::Extra1 => Some(0x08),
        PointerButton::Extra2 => Some(0x10),
    }
}

/// egui key → macro input, via the token names the CLI uses.
fn key_input(key: Key) -> Option<Input> {
    let debug = format!("{key:?}");
    let named = [
        ("ArrowDown", "downarrow"),
        ("ArrowLeft", "leftarrow"),
        ("ArrowRight", "rightarrow"),
        ("ArrowUp", "uparrow"),
        ("Escape", "esc"),
        ("Tab", "tab"),
        ("Backspace", "backspace"),
        ("Enter", "enter"),
        ("Space", "space"),
        ("Insert", "insert"),
        ("Delete", "delete"),
        ("Home", "home"),
        ("End", "end"),
        ("PageUp", "pageup"),
        ("PageDown", "pagedown"),
        ("Comma", "comma"),
        ("Backslash", "backslash"),
        ("Slash", "slash"),
        ("OpenBracket", "leftbrace"),
        ("CloseBracket", "rightbrace"),
        ("Backtick", "grave"),
        ("Minus", "minus"),
        ("Period", "dot"),
        ("Equals", "equal"),
        ("Semicolon", "semicolon"),
        ("Quote", "apostrophe"),
    ];
    let name = match named.iter().find(|(d, _)| *d == debug) {
        Some((_, n)) => (*n).to_string(),
        None => debug.strip_prefix("Num").unwrap_or(&debug).to_lowercase(),
    };
    macros::input(&name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_tokens() {
        assert_eq!(key_input(Key::A), macros::input("a").ok());
        assert_eq!(key_input(Key::Num7), macros::input("7").ok());
        assert_eq!(key_input(Key::F5), macros::input("f5").ok());
        assert_eq!(key_input(Key::ArrowUp), macros::input("uparrow").ok());
        assert_eq!(key_input(Key::Period), macros::input("dot").ok());
        assert_eq!(key_input(Key::F20), None);
    }

    #[test]
    fn captures_egui_input() {
        let ctx = egui::Context::default();
        let mut ed = MacroEditor::new();
        ed.recording = Some(Recorder {
            last: Instant::now(),
            mods: Modifiers::NONE,
        });
        let key = |pressed| Event::Key {
            key: Key::C,
            physical_key: Some(Key::C),
            pressed,
            repeat: false,
            modifiers: Modifiers::CTRL,
        };
        let raw = egui::RawInput {
            events: vec![
                Event::ModifiersChanged(Modifiers::CTRL),
                key(true),
                key(false),
                Event::ModifiersChanged(Modifiers::NONE),
            ],
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| ed.capture(ui)).textures_delta.clear();
        assert_eq!(macros::describe(&ed.doc.events), "down:ctrl c up:ctrl");
    }

    #[test]
    fn recording_inserts_and_times() {
        let mut ed = MacroEditor::new();
        ed.auto_delay = false;
        ed.fixed_delay = 40;
        ed.recording = Some(Recorder {
            last: Instant::now(),
            mods: Modifiers::NONE,
        });
        ed.record(true, Input::Key(0x04));
        ed.record(false, Input::Key(0x04));
        assert_eq!(ed.doc.events.len(), 2);
        assert!(ed.doc.events.iter().all(|e| e.delay == 40));
        // Insert before row 0, then rebind the pair to b.
        ed.selected = Some(0);
        ed.record(true, Input::Mod(1));
        assert_eq!(ed.doc.events[0].input, Input::Mod(1));
        ed.rebind_row(1, Input::Key(0x05));
        assert_eq!(ed.doc.events[2].input, Input::Key(0x05));
        assert!(check_name("a/b").is_err() && check_name("Копия").is_ok());
    }
}
