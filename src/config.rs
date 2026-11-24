//! Layout and encoding of the mouse's config memory, reverse-engineered from OemDrv.exe v3.1.
//!
//! Every field carries a checksum byte so that the field sums to 0x55:
//! a 1-byte value is stored as [v, 0x55-v], a 3-byte value as [a, b, c, 0x55-a-b-c].

/// Polling rate code.
pub const POLL: u16 = 0x00;
/// Number of DPI levels in the cycle.
pub const DPI_COUNT: u16 = 0x02;
/// Active DPI level, maintained by the mouse itself.
pub const DPI_CURRENT: u16 = 0x04;
/// DPI levels: 8 × [x_code, y_code, high_range_flag, cks].
pub const DPI: u16 = 0x0C;
/// DPI level colors: 8 × [r, g, b, cks].
pub const DPI_COLOR: u16 = 0x2C;
/// Logo LED: [mode, cks, brightness_code, cks, speed, cks].
pub const LOGO: u16 = 0x4C;
/// DPI indicator LED color: [r, g, b, cks], followed by [mode, cks, brightness_code, cks].
pub const DPI_LED: u16 = 0x54;
/// DPI indicator LED mode (inside the DPI_LED block).
pub const DPI_LED_MODE: u16 = 0x58;
/// DPI indicator LED speed: [speed, cks] (written only for animated modes).
pub const DPI_LED_SPEED: u16 = 0x5C;
/// Button actions: 16 × [type, a, b, cks].
pub const BUTTONS: u16 = 0x60;
/// Per-button keystroke blobs, 0x20 bytes each.
pub const KEY_BLOBS: u16 = 0x100;
/// Size of the header that holds every setting above.
pub const HEADER_LEN: usize = 0xA0;
/// Number of DPI level slots.
pub const LEVELS: usize = 8;
/// Number of button slots.
pub const SLOTS: usize = 16;
/// Memory slot (0-based) of each physical button, in the numbering of the mouse diagram
/// in OemDrv (button 1..8): left, right, wheel, side front, side rear, side middle, DPI+, DPI−.
pub const BUTTON_SLOTS: [usize; 8] = [0, 1, 2, 4, 3, 8, 10, 11];
/// Factory function of each physical button (Cfg.ini K*_1).
pub const BUTTON_FACTORY: [&str; 8] = [
    "left",
    "right",
    "middle",
    "forward",
    "back",
    "click:58:3",
    "dpi-up",
    "dpi-down",
];

/// LED brightness levels 1..=10 as stored in memory.
const BRIGHTNESS: [u8; 10] = [0x01, 0x1e, 0x3c, 0x5a, 0x80, 0x96, 0xb4, 0xd2, 0xe6, 0xff];

/// PAW3325 codes for 200..=5000 DPI, step 100.
const T3325: [u8; 49] = [
    0x04, 0x06, 0x08, 0x0b, 0x0d, 0x0f, 0x12, 0x14, 0x16, 0x19, 0x1b, 0x1d, 0x20, 0x22, 0x24, 0x27,
    0x29, 0x2b, 0x2e, 0x30, 0x32, 0x34, 0x37, 0x39, 0x3b, 0x3e, 0x40, 0x42, 0x45, 0x47, 0x49, 0x4c,
    0x4e, 0x50, 0x53, 0x55, 0x57, 0x5a, 0x5c, 0x5e, 0x61, 0x63, 0x65, 0x68, 0x6a, 0x6c, 0x6f, 0x71,
    0x73,
];

/// PAW3335 codes for 100..=16000 DPI, step 100.
const T3335: [u8; 160] = [
    0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x10, 0x11, 0x12,
    0x13, 0x14, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x23, 0x25, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x32, 0x33, 0x34, 0x35, 0x36, 0x38, 0x39,
    0x3a, 0x3b, 0x3c, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x44, 0x45, 0x46, 0x47, 0x48, 0x4a, 0x4b, 0x4c,
    0x4d, 0x4e, 0x50, 0x51, 0x52, 0x53, 0x54, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5c, 0x5d, 0x5e, 0x5f,
    0x60, 0x62, 0x63, 0x64, 0x65, 0x66, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6e, 0x6f, 0x70, 0x71, 0x72,
    0x74, 0x75, 0x76, 0x77, 0x78, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x80, 0x81, 0x82, 0x83, 0x84, 0x86,
    0x87, 0x88, 0x89, 0x8a, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x92, 0x93, 0x94, 0x95, 0x96, 0x98, 0x99,
    0x9a, 0x9b, 0x9c, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xaa, 0xab, 0xac,
    0xad, 0xae, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd,
];

/// Value that makes `bytes` + it sum to 0x55 (mod 256).
#[must_use]
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0x55u8, |s, b| s.wrapping_sub(*b))
}

/// [v, cks]
#[must_use]
pub fn pair(v: u8) -> [u8; 2] {
    [v, checksum(&[v])]
}

/// [a, b, c, cks]
#[must_use]
pub fn quad(v: [u8; 3]) -> [u8; 4] {
    [v[0], v[1], v[2], checksum(&v)]
}

/// Polling rate in Hz → stored code.
#[must_use]
pub fn poll_code(hz: u32) -> Option<u8> {
    match hz {
        1000 => Some(1),
        500 => Some(2),
        250 => Some(4),
        125 => Some(8),
        _ => None,
    }
}

/// Stored code → polling rate in Hz.
#[must_use]
pub fn poll_hz(code: u8) -> Option<u32> {
    [125, 250, 500, 1000]
        .into_iter()
        .find(|hz| poll_code(*hz) == Some(code))
}

/// LED brightness level 1..=10 → stored code.
#[must_use]
pub fn brightness_code(level: u8) -> Option<u8> {
    BRIGHTNESS.get(usize::from(level).checked_sub(1)?).copied()
}

/// Stored code → brightness level 1..=10 (OemDrv falls back to 1 too).
#[must_use]
pub fn brightness_level(code: u8) -> u8 {
    BRIGHTNESS
        .iter()
        .position(|c| *c == code)
        .map_or(1, |i| i as u8 + 1)
}

/// Optical sensor, selected by the receiver's bcdDevice >> 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    /// PixArt PAW3325: 200..=5000 native, 5200..=10000 via the high-range flag.
    Paw3325,
    /// PixArt PAW3335: 100..=16000.
    Paw3335,
}

impl Sensor {
    /// Sensor for a receiver bcdDevice, as OemDrv decides it.
    #[must_use]
    pub fn from_version(version: u16) -> Option<Self> {
        match version >> 12 {
            0 => Some(Self::Paw3325),
            1 => Some(Self::Paw3335),
            _ => None,
        }
    }

    /// Supported DPI values in human form.
    #[must_use]
    pub fn range(self) -> &'static str {
        match self {
            Self::Paw3325 => "200..5000 шаг 100, 5200..10000 шаг 200",
            Self::Paw3335 => "100..16000 шаг 100",
        }
    }

    /// DPI → (code, high-range flag).
    #[must_use]
    pub fn encode(self, dpi: u32) -> Option<(u8, bool)> {
        let at = |t: &[u8], first: u32, dpi: u32| {
            (dpi.is_multiple_of(100) && dpi >= first)
                .then(|| t.get(((dpi - first) / 100) as usize).copied())?
        };
        match self {
            Self::Paw3325 if dpi > 5000 => dpi
                .is_multiple_of(200)
                .then(|| at(&T3325, 200, dpi / 2))?
                .map(|c| (c, true)),
            Self::Paw3325 => at(&T3325, 200, dpi).map(|c| (c, false)),
            Self::Paw3335 => at(&T3335, 100, dpi).map(|c| (c, false)),
        }
    }

    /// (code, high-range flag) → DPI.
    #[must_use]
    pub fn decode(self, code: u8, high: bool) -> Option<u32> {
        let (t, first): (&[u8], u32) = match self {
            Self::Paw3325 => (&T3325, 200),
            Self::Paw3335 => (&T3335, 100),
        };
        let dpi = first + 100 * t.iter().position(|c| *c == code)? as u32;
        Some(if high { dpi * 2 } else { dpi })
    }
}

/// Modifier names in OemDrv's mask order.
pub(crate) const MODS: [(&str, u8); 4] = [
    ("ctrl", 0x01),
    ("shift", 0x02),
    ("alt", 0x04),
    ("win", 0x08),
];

/// Named HID keyboard usages besides letters, digits and F-keys.
const NAMED_KEYS: [(&str, u8); 28] = [
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2a),
    ("tab", 0x2b),
    ("space", 0x2c),
    ("minus", 0x2d),
    ("equal", 0x2e),
    ("leftbrace", 0x2f),
    ("rightbrace", 0x30),
    ("backslash", 0x31),
    ("semicolon", 0x33),
    ("apostrophe", 0x34),
    ("grave", 0x35),
    ("comma", 0x36),
    ("dot", 0x37),
    ("slash", 0x38),
    ("capslock", 0x39),
    ("printscreen", 0x46),
    ("insert", 0x49),
    ("home", 0x4a),
    ("pageup", 0x4b),
    ("delete", 0x4c),
    ("end", 0x4d),
    ("pagedown", 0x4e),
    ("rightarrow", 0x4f),
    ("leftarrow", 0x50),
    ("downarrow", 0x51),
    ("uparrow", 0x52),
];

pub(crate) fn key_usage(name: &str) -> Option<u8> {
    let b = name.as_bytes();
    match b {
        [c @ b'a'..=b'z'] => Some(c - b'a' + 0x04),
        [b'0'] => Some(0x27),
        [c @ b'1'..=b'9'] => Some(c - b'1' + 0x1e),
        [b'f', ..] if name.len() > 1 => match name[1..].parse::<u8>() {
            Ok(n @ 1..=12) => Some(0x3a + n - 1),
            _ => None,
        },
        _ => NAMED_KEYS.iter().find(|(n, _)| *n == name).map(|(_, u)| *u),
    }
}

pub(crate) fn key_name(usage: u8) -> String {
    match usage {
        0x04..=0x1d => char::from(b'a' + usage - 0x04).to_string(),
        0x1e..=0x26 => char::from(b'1' + usage - 0x1e).to_string(),
        0x27 => "0".into(),
        0x3a..=0x45 => format!("f{}", usage - 0x3a + 1),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, u)| *u == usage)
            .map_or_else(|| format!("{usage:#04x}"), |(n, _)| (*n).to_string()),
    }
}

/// Keystroke blob for a key button: [count, (op, code, 0)×count, cks].
/// Ops: 0x80 modifier down, 0x81 key down, 0x40 modifier up, 0x41 key up.
#[must_use]
pub fn key_blob(mods: u8, keys: &[u8]) -> Vec<u8> {
    let mod_bits = MODS.iter().map(|(_, m)| *m).filter(|m| mods & m != 0);
    let ev: Vec<[u8; 3]> = mod_bits
        .clone()
        .map(|m| [0x80, m, 0])
        .chain(keys.iter().map(|k| [0x81, *k, 0]))
        .chain(mod_bits.map(|m| [0x40, m, 0]))
        .chain(keys.iter().rev().map(|k| [0x41, *k, 0]))
        .collect();
    let mut out = vec![ev.len() as u8];
    out.extend(ev.concat());
    out.push(checksum(&out));
    out
}

fn describe_blob(blob: &[u8]) -> Option<String> {
    let n = usize::from(*blob.first()?);
    let body = blob.get(..1 + 3 * n + 1)?;
    if checksum(body) != 0 {
        return None;
    }
    let mut parts = Vec::new();
    for ev in body[1..1 + 3 * n].chunks(3) {
        match ev[0] {
            0x80 => parts.extend(
                MODS.iter()
                    .filter(|(_, m)| *m == ev[1])
                    .map(|(s, _)| (*s).to_string()),
            ),
            0x81 => parts.push(key_name(ev[1])),
            _ => {}
        }
    }
    Some(parts.join("+"))
}

/// A button action as stored in its slot, plus the keystroke blob for key actions.
pub struct Binding {
    /// [type, a, b] (checksum added on write).
    pub action: [u8; 3],
    /// Keystroke blob for type 5.
    pub blob: Option<Vec<u8>>,
}

/// Parses a user action: left, right, middle, back, forward, dpi-up, dpi-down, dpi-cycle,
/// off, double-click, triple-click, click:<interval>:<count>, key:ctrl+shift+a, raw:XXYYZZ.
pub fn parse_binding(s: &str) -> Result<Binding, String> {
    let plain = |action| Ok(Binding { action, blob: None });
    match s {
        "left" => plain([1, 0x01, 0]),
        "right" => plain([1, 0x02, 0]),
        "middle" => plain([1, 0x04, 0]),
        "back" => plain([1, 0x08, 0]),
        "forward" => plain([1, 0x10, 0]),
        "dpi-cycle" => plain([2, 1, 0]),
        "dpi-up" => plain([2, 2, 0]),
        "dpi-down" => plain([2, 3, 0]),
        "off" => plain([0, 0, 0]),
        "double-click" => plain([4, 0x32, 2]),
        "triple-click" => plain([4, 0x32, 3]),
        _ => {
            if let Some(rest) = s.strip_prefix("click:") {
                let (i, c) = rest
                    .split_once(':')
                    .ok_or("формат: click:<интервал>:<кол-во>")?;
                let num = |v: &str| v.parse::<u8>().map_err(|_| format!("не число 0..255: {v}"));
                plain([4, num(i)?, num(c)?])
            } else if let Some(combo) = s.strip_prefix("key:") {
                let (mut mods, mut keys) = (0u8, Vec::new());
                for part in combo.split('+') {
                    match MODS.iter().find(|(n, _)| *n == part) {
                        Some((_, m)) => mods |= m,
                        None => keys
                            .push(key_usage(part).ok_or(format!("неизвестная клавиша: {part}"))?),
                    }
                }
                if keys.is_empty() || keys.len() > 3 {
                    return Err("нужно от 1 до 3 обычных клавиш".into());
                }
                let blob = key_blob(mods, &keys);
                if blob.len() > 0x20 {
                    return Err("слишком длинная комбинация".into());
                }
                Ok(Binding {
                    action: [5, 0, 0],
                    blob: Some(blob),
                })
            } else if let Some(hex) = s.strip_prefix("raw:") {
                let v = u32::from_str_radix(hex, 16).map_err(|_| "формат: raw:XXYYZZ")?;
                let [_, a, b, c] = v.to_be_bytes();
                plain([a, b, c])
            } else {
                Err(format!("неизвестное действие: {s}"))
            }
        }
    }
}

/// Human description of a stored button action.
#[must_use]
pub fn describe_binding(action: [u8; 3], blob: &[u8]) -> String {
    match action {
        [0, 0, 0] => "off".into(),
        [1, 0x01, 0] => "left".into(),
        [1, 0x02, 0] => "right".into(),
        [1, 0x04, 0] => "middle".into(),
        [1, 0x08, 0] => "back".into(),
        [1, 0x10, 0] => "forward".into(),
        [2, 1, 0] => "dpi-cycle".into(),
        [2, 2, 0] => "dpi-up".into(),
        [2, 3, 0] => "dpi-down".into(),
        [4, 0x32, 2] => "double-click".into(),
        [4, 0x32, 3] => "triple-click".into(),
        [4, i, c] => format!("click:{i}:{c}"),
        [5, _, _] => {
            describe_blob(blob).map_or_else(|| "key:? (повреждено)".into(), |k| format!("key:{k}"))
        }
        [6, _, _] => "macro".into(),
        [a, b, c] => format!("raw:{a:02x}{b:02x}{c:02x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Header read from a real Renegade Wireless (PAW3325).
    const DUMP: [u8; 0x70] = [
        0x01, 0x54, 0x06, 0x4f, 0x03, 0x52, 0x00, 0x55, 0x00, 0x55, 0x00, 0x55, 0x2e, 0x2e, 0x00,
        0xf9, 0x73, 0x73, 0x01, 0x6e, 0x2e, 0x2e, 0x00, 0xf9, 0x73, 0x73, 0x01, 0x6e, 0x2e, 0x2e,
        0x00, 0xf9, 0x73, 0x73, 0x01, 0x6e, 0x73, 0x73, 0x00, 0x6f, 0x73, 0x73, 0x00, 0x6f, 0xff,
        0x00, 0xff, 0x57, 0x00, 0xff, 0xff, 0x57, 0xff, 0x00, 0xff, 0x57, 0x00, 0xff, 0xff, 0x57,
        0xff, 0x00, 0xff, 0x57, 0x00, 0xff, 0xff, 0x57, 0xff, 0x7d, 0x7d, 0x5c, 0x7d, 0x7d, 0xff,
        0x5c, 0x01, 0x54, 0x3c, 0x19, 0x03, 0x52, 0x00, 0x55, 0x00, 0xa2, 0x9a, 0x19, 0x01, 0x54,
        0x3c, 0x19, 0x03, 0x52, 0x00, 0x55, 0x01, 0x01, 0x00, 0x53, 0x01, 0x02, 0x00, 0x52, 0x01,
        0x04, 0x00, 0x50, 0x01, 0x08, 0x00, 0x4c,
    ];

    #[test]
    fn dump_fields_decode() {
        assert_eq!(poll_hz(DUMP[0]), Some(1000));
        assert_eq!(pair(DUMP[0]), [DUMP[0], DUMP[1]]);
        let s = Sensor::Paw3325;
        let dpi: Vec<_> = DUMP[0x0c..0x2c]
            .chunks(4)
            .map(|e| s.decode(e[0], e[2] == 1))
            .collect();
        let want = [2000, 10000, 2000, 10000, 2000, 10000, 5000, 5000];
        assert_eq!(dpi, want.map(Some));
        for e in DUMP[0x0c..0x4c].chunks(4).chain(DUMP[0x60..0x70].chunks(4)) {
            assert_eq!(quad([e[0], e[1], e[2]])[3], e[3]);
        }
        assert_eq!(brightness_level(DUMP[0x4e]), 3);
        assert_eq!(
            describe_binding([DUMP[0x6c], DUMP[0x6d], DUMP[0x6e]], &[]),
            "back"
        );
    }

    #[test]
    fn dpi_roundtrip() {
        for s in [Sensor::Paw3325, Sensor::Paw3335] {
            for dpi in (100..=16000).step_by(100) {
                if let Some((c, h)) = s.encode(dpi) {
                    assert_eq!(s.decode(c, h), Some(dpi));
                }
            }
        }
        assert_eq!(Sensor::Paw3325.encode(10000), Some((0x73, true)));
        assert_eq!(Sensor::Paw3325.encode(5100), None);
        assert_eq!(Sensor::Paw3325.encode(100), None);
        assert_eq!(Sensor::Paw3335.encode(16000), Some((0xbd, false)));
    }

    #[test]
    fn bindings_roundtrip() {
        for s in [
            "left",
            "back",
            "forward",
            "dpi-up",
            "off",
            "double-click",
            "click:58:3",
            "raw:070000",
        ] {
            let b = parse_binding(s).unwrap();
            assert_eq!(describe_binding(b.action, &[]), s);
        }
        let b = parse_binding("key:ctrl+shift+f5").unwrap();
        let blob = b.blob.unwrap();
        assert_eq!(blob[..4], [6, 0x80, 0x01, 0]);
        assert_eq!(checksum(&blob), 0);
        assert_eq!(describe_binding(b.action, &blob), "key:ctrl+shift+f5");
        assert!(parse_binding("key:ctrl+shift+alt+win+a+b+c").is_err());
        assert!(parse_binding("key:ctrl").is_err());
    }
}
