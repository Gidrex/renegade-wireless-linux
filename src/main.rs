//! Command-line tool for the ARDOR GAMING Renegade Wireless (replaces OemDrv.exe v3.1).

use std::env;
use std::fs;
use std::process::ExitCode;

use renegade::config::{BUTTON_SLOTS, LEVELS};
use renegade::device::Device;
use renegade::macros::{self, Repeat};
use renegade::mouse::{self, BACKUP_LEN, Button, DPI_LED_MODES, LOGO_MODES, Led, Res, State};

const USAGE: &str = "\
renegade — настройка ARDOR GAMING Renegade Wireless под Linux (графическая версия: renegade-gui)

  renegade show                              текущие настройки
  renegade polling <125|250|500|1000>        частота опроса, Гц
  renegade dpi <dpi1> [dpi2 .. dpi8]         уровни DPI (их число = длина цикла)
  renegade dpi-color <уровень 1-8> <RRGGBB>  цвет уровня DPI
  renegade logo <off|static|breathing> [brightness=1-10] [speed=N]
  renegade dpi-led <off|static|breathing|neon> [color=RRGGBB] [brightness=1-10] [speed=N]
  renegade button <1-8> <действие>           переназначить кнопку (номера как на схеме в OemDrv)
  renegade macro <1-8> <повтор> <события…|@файл> [name=имя]  записать макрос на кнопку
  renegade backup <файл>                     сохранить память мыши
  renegade restore <файл>                    восстановить из backup

Действия кнопок: left right middle back forward dpi-up dpi-down dpi-cycle off
  double-click triple-click click:<интервал>:<кол-во> key:ctrl+shift+a raw:XXYYZZ

Макросы: повтор = 1..253 раз | hold (пока кнопка нажата) | until-press (до нажатия любой кнопки)
  события: a  ctrl+c (нажать по порядку, отпустить в обратном)  down:shift  up:shift
           left right middle back forward (кнопки мыши)  50ms (пауза после предыдущего, иначе 3ms)
  пример: renegade macro 5 1 ctrl+a 20ms ctrl+c
          renegade macro 6 hold left 30ms           (автокликер)
          renegade macro 4 1 @~/.config/renegade/macros/copy.txt   (из библиотеки GUI)
  кнопки: 1 левая, 2 правая, 3 колесо, 4/5/6 боковые, 7 DPI+, 8 DPI−
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ошибка: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[&str]) -> Res<()> {
    let Some((&cmd, rest)) = args.split_first() else {
        print!("{USAGE}");
        return Ok(());
    };
    if matches!(cmd, "help" | "-h" | "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let mut dev = Device::open()?;
    match (cmd, rest) {
        ("show", []) => show(&State::read(&mut dev)?),
        ("polling", [hz]) => {
            mouse::set_polling(&mut dev, hz.parse().map_err(|_| "частота: число")?)
        }
        ("dpi", levels) if !levels.is_empty() => {
            let sensor = State::read(&mut dev)?.sensor;
            let levels = levels
                .iter()
                .map(|l| l.parse())
                .collect::<Result<Vec<u32>, _>>();
            mouse::set_dpi(&mut dev, sensor, &levels.map_err(|_| "DPI: числа")?)
        }
        ("dpi-color", [level, color]) => {
            mouse::set_dpi_color(&mut dev, parse_index(level, LEVELS)?, parse_rgb(color)?)
        }
        ("logo", [mode, opts @ ..]) => {
            let o = Opts::parse(opts)?;
            if o.color.is_some() {
                return Err("у логотипа нет настройки цвета".into());
            }
            let led = o.apply(
                State::read(&mut dev)?.logo(),
                mode_index(&LOGO_MODES, mode)?,
            );
            mouse::set_logo(&mut dev, led)
        }
        ("dpi-led", [mode, opts @ ..]) => {
            let o = Opts::parse(opts)?;
            let (cur, rgb) = State::read(&mut dev)?.dpi_led();
            let led = o.apply(cur, mode_index(&DPI_LED_MODES, mode)?);
            mouse::set_dpi_led(&mut dev, led, o.color.unwrap_or(rgb))
        }
        ("button", [n, action]) => mouse::set_button(&mut dev, button_slot(n)?, action),
        ("macro", [n, repeat, args @ ..]) if !args.is_empty() => {
            let (name, text) = macro_source(args)?;
            let tokens: Vec<&str> = text.split_whitespace().collect();
            mouse::set_macro(
                &mut dev,
                button_slot(n)?,
                Repeat::parse(repeat)?,
                &name,
                &tokens,
            )
        }
        ("backup", [file]) => {
            fs::write(file, mouse::backup(&mut dev)?)?;
            println!("сохранено {BACKUP_LEN} байт в {file}");
            Ok(())
        }
        ("restore", [file]) => mouse::restore(&mut dev, &fs::read(file)?),
        _ => Err("неверная команда, см. `renegade help`".into()),
    }?;
    if cmd != "show" && cmd != "backup" {
        println!("готово");
    }
    Ok(())
}

fn show(s: &State) -> Res<()> {
    println!(
        "Сенсор: {:?} ({}), bcdDevice {:#06x}",
        s.sensor,
        s.sensor.range(),
        s.version
    );
    match s.poll_hz() {
        Some(hz) => println!("Частота опроса: {hz} Гц"),
        None => println!("Частота опроса: ?"),
    }
    println!(
        "DPI: уровней в цикле {}, текущий (сырое значение) {}",
        s.dpi_count(),
        s.current_level()
    );
    for i in 0..LEVELS {
        let dpi = match s.dpi(i) {
            Some((x, y)) if x == y => x.to_string(),
            Some((x, y)) => format!("{x}x{y}"),
            None => "?".into(),
        };
        let off = if i < s.dpi_count() {
            ""
        } else {
            " (не в цикле)"
        };
        println!("  {}: {dpi:>6}  цвет {}{off}", i + 1, hex(&s.color(i)));
    }
    let l = s.logo();
    print!("Подсветка логотипа: {}", mode_name(&LOGO_MODES, l.mode));
    if l.mode != 0 {
        print!(", яркость {}/10, скорость {}", l.brightness, l.speed);
    }
    let (d, rgb) = s.dpi_led();
    print!("\nDPI-индикатор: {}", mode_name(&DPI_LED_MODES, d.mode));
    if d.mode != 0 {
        print!(
            ", цвет {}, яркость {}/10, скорость {}",
            hex(&rgb),
            d.brightness,
            d.speed
        );
    }
    println!("\nКнопки:");
    for (i, &slot) in BUTTON_SLOTS.iter().enumerate() {
        let text = match &s.buttons[slot] {
            Button::Action(a) => a.clone(),
            Button::Macro {
                repeat,
                name,
                events,
            } => {
                let events = events.as_deref().unwrap_or("? (повреждён)");
                format!("macro {} «{name}»: {events}", repeat.name())
            }
        };
        println!("  {}: {text}", i + 1);
    }
    Ok(())
}

/// Optional `key=value` LED parameters.
#[derive(Default)]
struct Opts {
    color: Option<[u8; 3]>,
    brightness: Option<u8>,
    speed: Option<u8>,
}

impl Opts {
    fn parse(args: &[&str]) -> Res<Self> {
        let mut o = Self::default();
        for a in args {
            let (k, v) = a
                .split_once('=')
                .ok_or_else(|| format!("ожидалось ключ=значение: {a}"))?;
            let num = || {
                v.parse::<u8>()
                    .map_err(|_| format!("{k}: нужно число 0..255"))
            };
            match k {
                "color" => o.color = Some(parse_rgb(v)?),
                "brightness" => o.brightness = Some(num()?),
                "speed" => o.speed = Some(num()?),
                _ => return Err(format!("неизвестный параметр: {k}").into()),
            }
        }
        Ok(o)
    }

    /// The current LED with the given mode and any overridden parameters.
    fn apply(&self, cur: Led, mode: u8) -> Led {
        Led {
            mode,
            brightness: self.brightness.unwrap_or(cur.brightness),
            speed: self.speed.unwrap_or(cur.speed),
        }
    }
}

fn mode_index(modes: &[&str], mode: &str) -> Res<u8> {
    let i = modes
        .iter()
        .position(|m| *m == mode)
        .ok_or_else(|| format!("режим: {}", modes.join(", ")))?;
    Ok(i as u8)
}

fn mode_name(modes: &[&str], code: u8) -> String {
    modes
        .get(usize::from(code))
        .map_or_else(|| format!("? (код {code})"), |m| (*m).to_string())
}

/// Button number 1..8 (as on the OemDrv mouse diagram) → memory slot.
fn button_slot(s: &str) -> Res<usize> {
    Ok(BUTTON_SLOTS[parse_index(s, BUTTON_SLOTS.len())?])
}

/// Macro name and event tokens: `@file` from the library, or inline tokens with an optional `name=…`.
fn macro_source(args: &[&str]) -> Res<(String, String)> {
    if let [file] = args
        && let Some(path) = file.strip_prefix('@')
    {
        let path = match path.strip_prefix("~/") {
            Some(rest) => std::path::PathBuf::from(env::var("HOME")?).join(rest),
            None => path.into(),
        };
        return Ok(macros::load_file(&path)?);
    }
    let (names, events): (Vec<&str>, Vec<&str>) = args.iter().partition(|a| a.starts_with("name="));
    let name = names.last().map_or("", |n| &n["name=".len()..]);
    Ok((name.to_string(), events.join(" ")))
}

/// 1-based index from the command line → 0-based.
fn parse_index(s: &str, max: usize) -> Res<usize> {
    match s.parse::<usize>() {
        Ok(n @ 1..) if n <= max => Ok(n - 1),
        _ => Err(format!("номер: 1..{max}").into()),
    }
}

fn parse_rgb(s: &str) -> Res<[u8; 3]> {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16)
        .ok()
        .filter(|_| h.len() == 6)
        .ok_or("цвет: RRGGBB")?;
    let [_, r, g, b] = v.to_be_bytes();
    Ok([r, g, b])
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_parsing() {
        assert_eq!(parse_rgb("#00a29A").unwrap(), [0x00, 0xa2, 0x9a]);
        assert!(parse_rgb("fff").is_err());
        assert_eq!(parse_index("8", LEVELS).unwrap(), 7);
        assert!(parse_index("0", LEVELS).is_err());
        assert_eq!(button_slot("4").unwrap(), 4);
        assert!(button_slot("9").is_err());
        let (name, ev) = macro_source(&["a", "name=Копия", "50ms"]).unwrap();
        assert_eq!((name.as_str(), ev.as_str()), ("Копия", "a 50ms"));
        let o = Opts::parse(&["color=ff0000", "speed=3"]).unwrap();
        assert_eq!(
            (o.color, o.speed, o.brightness),
            (Some([255, 0, 0]), Some(3), None)
        );
        let led = o.apply(
            Led {
                mode: 1,
                brightness: 4,
                speed: 9,
            },
            2,
        );
        assert_eq!(
            led,
            Led {
                mode: 2,
                brightness: 4,
                speed: 3
            }
        );
    }
}
