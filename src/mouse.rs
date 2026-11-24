//! Settings as a whole: decoding them from the mouse and writing single changes back.

use std::error::Error;

use crate::config::{
    BUTTONS, DPI, DPI_COLOR, DPI_COUNT, DPI_CURRENT, DPI_LED, DPI_LED_MODE, DPI_LED_SPEED,
    HEADER_LEN, KEY_BLOBS, LEVELS, LOGO, POLL, SLOTS, Sensor, brightness_code, brightness_level,
    describe_binding, pair, parse_binding, poll_code, poll_hz, quad,
};
use crate::device::Device;
use crate::macros::{self, Repeat};

/// Result with a boxed error, as used across the crate.
pub type Res<T> = Result<T, Box<dyn Error>>;

/// Config memory span saved by `backup`: settings, key blobs and all 16 macro areas.
pub const BACKUP_LEN: usize = macros::addr(SLOTS) as usize;
/// Logo LED modes, by stored code.
pub const LOGO_MODES: [&str; 3] = ["off", "static", "breathing"];
/// DPI indicator LED modes, by stored code.
pub const DPI_LED_MODES: [&str; 4] = ["off", "static", "breathing", "neon"];

/// An LED's mode code, brightness level 1..=10 and raw speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Led {
    /// Index into `LOGO_MODES` / `DPI_LED_MODES`; 0 is off.
    pub mode: u8,
    /// Brightness level 1..=10.
    pub brightness: u8,
    /// Animation speed as stored.
    pub speed: u8,
}

/// What a button does, in the text forms the CLI accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Button {
    /// `parse_binding` syntax: left, key:ctrl+c, click:50:2, raw:…
    Action(String),
    /// Macro with its repeat mode and `macros::parse` tokens (None if the stored data is corrupt).
    Macro {
        /// Repeat mode.
        repeat: Repeat,
        /// Name stored with the macro (shown by the Windows tool).
        name: String,
        /// Event tokens.
        events: Option<String>,
    },
}

/// Everything readable from the mouse.
pub struct State {
    /// Sensor model.
    pub sensor: Sensor,
    /// Receiver bcdDevice.
    pub version: u16,
    header: Vec<u8>,
    /// All 16 button slots.
    pub buttons: Vec<Button>,
}

impl State {
    /// Reads settings, key blobs and macros of bound buttons.
    pub fn read(dev: &mut Device) -> Res<Self> {
        let sensor = Sensor::from_version(dev.version)
            .ok_or_else(|| format!("неизвестный сенсор (bcdDevice {:#06x})", dev.version))?;
        let header = dev.read_mem(0, HEADER_LEN)?;
        let mut buttons = Vec::with_capacity(SLOTS);
        for slot in 0..SLOTS {
            let a = &header[usize::from(BUTTONS) + 4 * slot..][..3];
            let action = [a[0], a[1], a[2]];
            buttons.push(match action[0] {
                5 => Button::Action(describe_binding(
                    action,
                    &dev.read_mem(at(KEY_BLOBS, 0x20 * slot), 0x20)?,
                )),
                6 => {
                    let base = macros::addr(slot);
                    let count = dev.read_mem(at(base, macros::COUNT_OFFSET), 1)?[0];
                    let data = dev.read_mem(base, macros::stored_len(count))?;
                    let events = macros::decode(&data).map(|e| macros::describe(&e));
                    Button::Macro {
                        repeat: Repeat::from_code(action[2]),
                        name: macros::decode_name(&data),
                        events,
                    }
                }
                _ => Button::Action(describe_binding(action, &[])),
            });
        }
        Ok(Self {
            sensor,
            version: dev.version,
            header,
            buttons,
        })
    }

    fn byte(&self, base: u16, off: usize) -> u8 {
        self.header[usize::from(base) + off]
    }

    /// Polling rate, None if the stored code is unknown.
    #[must_use]
    pub fn poll_hz(&self) -> Option<u32> {
        poll_hz(self.byte(POLL, 0))
    }

    /// Number of DPI levels in the cycle.
    #[must_use]
    pub fn dpi_count(&self) -> usize {
        usize::from(self.byte(DPI_COUNT, 0)).min(LEVELS)
    }

    /// Active DPI level as the mouse stores it.
    #[must_use]
    pub fn current_level(&self) -> u8 {
        self.byte(DPI_CURRENT, 0)
    }

    /// (x, y) DPI of a 0-based level slot.
    #[must_use]
    pub fn dpi(&self, level: usize) -> Option<(u32, u32)> {
        let high = self.byte(DPI, 4 * level + 2) == 1;
        let x = self.sensor.decode(self.byte(DPI, 4 * level), high)?;
        Some((x, self.sensor.decode(self.byte(DPI, 4 * level + 1), high)?))
    }

    /// Color of a 0-based level slot.
    #[must_use]
    pub fn color(&self, level: usize) -> [u8; 3] {
        [0, 1, 2].map(|c| self.byte(DPI_COLOR, 4 * level + c))
    }

    /// Logo LED.
    #[must_use]
    pub fn logo(&self) -> Led {
        Led {
            mode: self.byte(LOGO, 0),
            brightness: brightness_level(self.byte(LOGO, 2)),
            speed: self.byte(LOGO, 4),
        }
    }

    /// DPI indicator LED and its color.
    #[must_use]
    pub fn dpi_led(&self) -> (Led, [u8; 3]) {
        let led = Led {
            mode: self.byte(DPI_LED, 4),
            brightness: brightness_level(self.byte(DPI_LED, 6)),
            speed: self.byte(DPI_LED, 8),
        };
        (led, [0, 1, 2].map(|c| self.byte(DPI_LED, c)))
    }
}

/// Sets the polling rate in Hz.
pub fn set_polling(dev: &mut Device, hz: u32) -> Res<()> {
    let code = poll_code(hz).ok_or("частота: 125, 250, 500 или 1000")?;
    dev.write_mem(POLL, &pair(code))
}

/// Sets the DPI cycle to `levels` (1..=8 values).
pub fn set_dpi(dev: &mut Device, sensor: Sensor, levels: &[u32]) -> Res<()> {
    if levels.is_empty() || levels.len() > LEVELS {
        return Err(format!("нужно от 1 до {LEVELS} уровней DPI").into());
    }
    let mut data = Vec::new();
    for &dpi in levels {
        let (code, high) = sensor
            .encode(dpi)
            .ok_or_else(|| format!("DPI {dpi} не поддерживается: {}", sensor.range()))?;
        data.extend(quad([code, code, u8::from(high)]));
    }
    dev.write_mem(DPI, &data)?;
    dev.write_mem(DPI_COUNT, &pair(levels.len() as u8))
}

/// Sets the color of a 0-based DPI level.
pub fn set_dpi_color(dev: &mut Device, level: usize, rgb: [u8; 3]) -> Res<()> {
    dev.write_mem(at(DPI_COLOR, 4 * level), &quad(rgb))
}

/// Writes the logo LED.
pub fn set_logo(dev: &mut Device, led: Led) -> Res<()> {
    if usize::from(led.mode) >= LOGO_MODES.len() {
        return Err("неизвестный режим логотипа".into());
    }
    if led.mode == 0 {
        return dev.write_mem(LOGO, &pair(0));
    }
    let data = [pair(led.mode), pair(bright(led)?), pair(led.speed)].concat();
    dev.write_mem(LOGO, &data)
}

/// Writes the DPI indicator LED.
pub fn set_dpi_led(dev: &mut Device, led: Led, rgb: [u8; 3]) -> Res<()> {
    if usize::from(led.mode) >= DPI_LED_MODES.len() {
        return Err("неизвестный режим DPI-индикатора".into());
    }
    if led.mode == 0 {
        return dev.write_mem(DPI_LED_MODE, &pair(0));
    }
    let data = [&quad(rgb)[..], &pair(led.mode), &pair(bright(led)?)].concat();
    dev.write_mem(DPI_LED, &data)?;
    // OemDrv writes speed only for the animated modes.
    if led.mode >= 2 {
        dev.write_mem(DPI_LED_SPEED, &pair(led.speed))?;
    }
    Ok(())
}

fn bright(led: Led) -> Res<u8> {
    brightness_code(led.brightness).ok_or_else(|| "яркость: 1..10".into())
}

/// Binds a 0-based button slot to a `parse_binding` action.
pub fn set_button(dev: &mut Device, slot: usize, action: &str) -> Res<()> {
    let b = parse_binding(action)?;
    ensure_left_kept(dev, slot, b.action)?;
    if let Some(blob) = &b.blob {
        dev.write_mem(at(KEY_BLOBS, 0x20 * slot), blob)?;
    }
    dev.write_mem(at(BUTTONS, 4 * slot), &quad(b.action))
}

/// Writes a named macro to a 0-based button slot and binds the button to it.
pub fn set_macro(
    dev: &mut Device,
    slot: usize,
    repeat: Repeat,
    name: &str,
    tokens: &[&str],
) -> Res<()> {
    let events = macros::parse(tokens)?;
    // OemDrv binds a macro as [6, slot, repeat].
    let action = [6, slot as u8, repeat.code()];
    ensure_left_kept(dev, slot, action)?;
    dev.write_mem(macros::addr(slot), &macros::encode(&events, name))?;
    dev.write_mem(at(BUTTONS, 4 * slot), &quad(action))
}

/// Refuses a change that would leave no button acting as the left click.
fn ensure_left_kept(dev: &mut Device, slot: usize, new_action: [u8; 3]) -> Res<()> {
    let table = dev.read_mem(BUTTONS, 4 * SLOTS)?;
    let left = |i: usize, a: &[u8]| {
        if i == slot {
            new_action == [1, 1, 0]
        } else {
            a[..3] == [1, 1, 0]
        }
    };
    if table.chunks(4).enumerate().any(|(i, a)| left(i, a)) {
        Ok(())
    } else {
        Err("хотя бы одна кнопка должна остаться левой (left)".into())
    }
}

/// Reads the whole config memory.
pub fn backup(dev: &mut Device) -> Res<Vec<u8>> {
    dev.read_mem(0, BACKUP_LEN)
}

/// Writes back a `backup` image (or a shorter prefix from older versions), only where it differs.
pub fn restore(dev: &mut Device, image: &[u8]) -> Res<()> {
    if image.is_empty() || image.len() > BACKUP_LEN {
        return Err(format!("ожидалось до {BACKUP_LEN} байт, а не {}", image.len()).into());
    }
    let have = dev.read_mem(0, image.len())?;
    for (i, (w, h)) in image.chunks(10).zip(have.chunks(10)).enumerate() {
        if w != h {
            dev.write_mem(at(0, 10 * i), w)?;
        }
    }
    Ok(())
}

fn at(base: u16, off: usize) -> u16 {
    base + off as u16
}
