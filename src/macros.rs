//! Macro storage, reverse-engineered from OemDrv.exe and checked against macros on a real mouse.
//!
//! Each button slot owns 0x180 bytes at 0x300 + slot*0x180:
//! `[0]` name length in bytes, `[1..31]` UTF-16LE name (only for the Windows UI), `[0x1f]` event count,
//! then 5-byte events `[op, code, 0, delay_hi, delay_lo]`, a checksum over `[0x1f..]` and 3 zero bytes.
//! `op` = 0x80 (press) or 0x40 (release) | kind: 0 modifier bit, 1 HID key usage, 4 mouse button mask.
//! The button itself is bound as `[6, slot, repeat]`.

use crate::config::{MODS, checksum, key_name, key_usage};

/// First macro area.
const MACROS: u16 = 0x300;
/// Bytes reserved per macro.
pub const MACRO_LEN: usize = 0x180;
const HEADER: usize = 0x20;
/// Most events that fit into one macro area.
pub const MAX_EVENTS: usize = (MACRO_LEN - HEADER - 4) / 5;
/// OemDrv never stores a delay below 3 ms.
const MIN_DELAY: u16 = 3;
/// Mouse buttons usable inside macros (same masks as button bindings).
const MOUSE: [(&str, u8); 5] = [
    ("left", 0x01),
    ("right", 0x02),
    ("middle", 0x04),
    ("back", 0x08),
    ("forward", 0x10),
];
/// Right-hand modifiers (left-hand ones come from `MODS`).
const RIGHT_MODS: [(&str, u8); 3] = [("rctrl", 0x10), ("rshift", 0x20), ("ralt", 0x40)];

/// What a macro event presses or releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Modifier bit (ctrl 0x01, shift 0x02, alt 0x04, win 0x08, right-hand ones shifted by 4).
    Mod(u8),
    /// HID keyboard usage.
    Key(u8),
    /// Mouse button mask.
    Mouse(u8),
}

/// One press or release followed by a delay in ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// Press (true) or release (false).
    pub down: bool,
    /// What is pressed or released.
    pub input: Input,
    /// Pause after this event, ms.
    pub delay: u16,
}

/// How a macro repeats; stored as the third byte of the button binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// Play N times (1..=253).
    Times(u8),
    /// Loop while the button is held.
    Hold,
    /// Loop until any button is pressed.
    UntilPress,
}

impl Repeat {
    /// Parses `N`, `hold` or `until-press`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "hold" => Ok(Self::Hold),
            "until-press" => Ok(Self::UntilPress),
            _ => match s.parse::<u8>() {
                Ok(n @ 1..=0xfd) => Ok(Self::Times(n)),
                _ => Err("повтор: число 1..253, hold или until-press".into()),
            },
        }
    }

    /// Stored byte.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::Times(n) => n,
            Self::Hold => 0xff,
            Self::UntilPress => 0xfe,
        }
    }

    /// From the stored byte.
    #[must_use]
    pub fn from_code(c: u8) -> Self {
        match c {
            0xff => Self::Hold,
            0xfe => Self::UntilPress,
            n => Self::Times(n),
        }
    }

    /// Text form accepted by `parse`.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Times(n) => n.to_string(),
            Self::Hold => "hold".into(),
            Self::UntilPress => "until-press".into(),
        }
    }
}

/// Memory address of the macro for a 0-based button slot.
#[must_use]
pub const fn addr(slot: usize) -> u16 {
    MACROS + (slot * MACRO_LEN) as u16
}

/// Bytes to read to decode a stored macro of `count` events.
#[must_use]
pub fn stored_len(count: u8) -> usize {
    HEADER + 5 * usize::from(count) + 1
}

/// Offset of the event count inside a macro area.
pub const COUNT_OFFSET: usize = HEADER - 1;

/// Serialises events into a macro area image (without the name, which only the Windows UI shows).
#[must_use]
pub fn encode(events: &[Event]) -> Vec<u8> {
    let mut b = vec![0u8; HEADER];
    b[COUNT_OFFSET] = events.len() as u8;
    for e in events {
        let (kind, code) = match e.input {
            Input::Mod(m) => (0, m),
            Input::Key(k) => (1, k),
            Input::Mouse(m) => (4, m),
        };
        b.extend([if e.down { 0x80 } else { 0x40 } | kind, code, 0]);
        b.extend(e.delay.max(MIN_DELAY).to_be_bytes());
    }
    let cks = checksum(&b[COUNT_OFFSET..]);
    b.push(cks);
    b.extend([0, 0, 0]);
    b
}

/// Parses a stored macro area image; None if it is empty or corrupt.
#[must_use]
pub fn decode(data: &[u8]) -> Option<Vec<Event>> {
    let n = *data.get(COUNT_OFFSET)?;
    let body = data.get(COUNT_OFFSET..stored_len(n))?;
    if n == 0 || checksum(body) != 0 {
        return None;
    }
    body[1..body.len() - 1]
        .chunks(5)
        .map(|ev| {
            let input = match ev[0] & 0x3f {
                0 => Input::Mod(ev[1]),
                1 => Input::Key(ev[1]),
                4 => Input::Mouse(ev[1]),
                _ => return None,
            };
            Some(Event {
                down: ev[0] & 0x80 != 0,
                input,
                delay: u16::from_be_bytes([ev[3], ev[4]]),
            })
        })
        .collect()
}

fn input(name: &str) -> Result<Input, String> {
    let find = |table: &[(&str, u8)]| table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    find(&MOUSE)
        .map(Input::Mouse)
        .or_else(|| find(&MODS).or_else(|| find(&RIGHT_MODS)).map(Input::Mod))
        .or_else(|| key_usage(name).map(Input::Key))
        .ok_or_else(|| format!("неизвестная клавиша: {name}"))
}

fn input_name(i: Input) -> String {
    let find = |table: &[(&'static str, u8)], v: u8| {
        table
            .iter()
            .find(|(_, x)| *x == v)
            .map(|(n, _)| (*n).to_string())
    };
    match i {
        Input::Mouse(m) => find(&MOUSE, m).unwrap_or_else(|| format!("mouse{m:#04x}")),
        Input::Mod(m) => find(&MODS, m)
            .or_else(|| find(&RIGHT_MODS, m))
            .unwrap_or_else(|| format!("mod{m:#04x}")),
        Input::Key(k) => key_name(k),
    }
}

/// Parses macro tokens: `a`, `ctrl+c` (press in order, release in reverse), `down:x`, `up:x`,
/// and `50ms` (pause after the previous event; 3 ms otherwise).
pub fn parse(tokens: &[&str]) -> Result<Vec<Event>, String> {
    let at = |down, input| Event {
        down,
        input,
        delay: MIN_DELAY,
    };
    let mut ev: Vec<Event> = Vec::new();
    for t in tokens {
        if let Some(ms) = t.strip_suffix("ms") {
            let d = ms
                .parse::<u16>()
                .map_err(|_| format!("задержка 0..65535 мс: {t}"))?;
            ev.last_mut()
                .ok_or("задержка не может стоять первой")?
                .delay = d;
        } else if let Some(k) = t.strip_prefix("down:") {
            ev.push(at(true, input(k)?));
        } else if let Some(k) = t.strip_prefix("up:") {
            ev.push(at(false, input(k)?));
        } else {
            let chord = t.split('+').map(input).collect::<Result<Vec<_>, _>>()?;
            ev.extend(chord.iter().map(|i| at(true, *i)));
            ev.extend(chord.iter().rev().map(|i| at(false, *i)));
        }
    }
    if ev.is_empty() || ev.len() > MAX_EVENTS {
        return Err(format!(
            "в макросе должно быть от 1 до {MAX_EVENTS} нажатий/отпусканий"
        ));
    }
    Ok(ev)
}

/// Token form of events, accepted back by `parse`.
#[must_use]
pub fn describe(events: &[Event]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(e) = events.get(i) {
        let next = events.get(i + 1);
        let tap =
            e.down && e.delay == MIN_DELAY && next.is_some_and(|n| !n.down && n.input == e.input);
        let (token, last) = match (tap, next) {
            (true, Some(up)) => (input_name(e.input), up),
            _ => (
                format!(
                    "{}:{}",
                    if e.down { "down" } else { "up" },
                    input_name(e.input)
                ),
                e,
            ),
        };
        out.push(token);
        if last.delay != MIN_DELAY {
            out.push(format!("{}ms", last.delay));
        }
        i += if tap { 2 } else { 1 };
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Macro "lkm" read from a real mouse: left down, left up; stale bytes follow.
    const LKM: [u8; 0x34] = [
        0x06, 0x6c, 0x00, 0x6b, 0x00, 0x6d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x02, 0x84, 0x01, 0x00, 0x00, 0x03, 0x44, 0x01, 0x00, 0x00, 0x03, 0x83, 0x00, 0x00,
        0x00, 0x03, 0x44, 0x01, 0x00, 0x00, 0x03,
    ];

    #[test]
    fn decodes_real_macro() {
        let ev = decode(&LKM).unwrap();
        assert_eq!(describe(&ev), "left");
        // Our encoding of the same events matches the device bytes (name aside).
        assert_eq!(
            encode(&ev)[COUNT_OFFSET..COUNT_OFFSET + 12],
            LKM[COUNT_OFFSET..COUNT_OFFSET + 12]
        );
    }

    #[test]
    fn roundtrip() {
        for src in [
            "ctrl+c 50ms ctrl+v",
            "down:shift a 1000ms up:shift",
            "left 10ms right",
            "rctrl+f12 uparrow",
        ] {
            let ev = parse(&src.split(' ').collect::<Vec<_>>()).unwrap();
            let img = encode(&ev);
            assert!(img.len() <= MACRO_LEN);
            let back = decode(&img).unwrap();
            assert_eq!(back, ev);
            assert_eq!(
                parse(&describe(&back).split(' ').collect::<Vec<_>>()).unwrap(),
                ev
            );
        }
        assert_eq!(
            describe(&parse(&["ctrl+c"]).unwrap()),
            "down:ctrl c up:ctrl"
        );
    }

    #[test]
    fn limits() {
        assert!(parse(&["50ms"]).is_err());
        assert!(parse(&["nosuchkey"]).is_err());
        assert!(parse(&vec!["down:a"; MAX_EVENTS + 1]).is_err());
        let full = encode(&parse(&vec!["down:a"; MAX_EVENTS]).unwrap());
        assert!(full.len() <= MACRO_LEN && full.len() + 5 > MACRO_LEN);
        assert_eq!(Repeat::parse("hold").unwrap().code(), 0xff);
        assert!(Repeat::parse("254").is_err());
        assert_eq!(Repeat::from_code(0xfe), Repeat::UntilPress);
    }
}
