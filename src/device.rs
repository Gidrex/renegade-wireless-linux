//! Raw HID transport to the 2.4G receiver (25a7:fa4a / fa4b) via Linux hidraw.
//!
//! Commands go out as 17-byte feature report 0x08, replies come back as input report 0x09.
//! Byte 16 of both is a checksum: the 17 bytes sum to 0x55.

use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::config::checksum;

const VID: u32 = 0x25A7;
const PIDS: [u32; 2] = [0xFA4A, 0xFA4B];
const CMD_WRITE: u8 = 0x07;
const CMD_READ: u8 = 0x08;
const CHUNK: usize = 10;
// An awake mouse answers over the radio in well under a second.
const TIMEOUT: Duration = Duration::from_millis(1000);
// The mouse sleeps a few seconds after the last movement and ignores commands until moved.
const WAKE_WAIT: Duration = Duration::from_secs(30);

type Res<T> = Result<T, Box<dyn Error>>;
type Reply = [u8; 17];

/// An opened receiver: vendor hidraw node plus its USB bcdDevice (encodes the sensor).
pub struct Device {
    file: File,
    /// USB bcdDevice of the receiver; the top nibble selects the sensor.
    pub version: u16,
}

impl Device {
    /// Finds the vendor-config hidraw node of the receiver and opens it.
    pub fn open() -> Res<Self> {
        for entry in fs::read_dir("/sys/class/hidraw")? {
            let entry = entry?;
            let dev = entry.path().join("device");
            let uevent = fs::read_to_string(dev.join("uevent")).unwrap_or_default();
            if !is_receiver(&uevent) {
                continue;
            }
            // The config collection is the one with usage page 0xFF02 (feature report 0x08).
            let desc = fs::read(dev.join("report_descriptor")).unwrap_or_default();
            if !desc.windows(3).any(|w| w == [0x06, 0x02, 0xff]) {
                continue;
            }
            let node = Path::new("/dev").join(entry.file_name());
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&node)
                .map_err(|e| {
                    format!(
                        "{}: {e} (нужны права на hidraw, см. README)",
                        node.display()
                    )
                })?;
            return Ok(Self {
                file,
                version: bcd_device(&dev).unwrap_or(0),
            });
        }
        Err("приёмник 25a7:fa4a не найден".into())
    }

    /// Reads `len` bytes of the mouse's config memory starting at `addr`.
    pub fn read_mem(&mut self, addr: u16, len: usize) -> Res<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        for (i, a) in chunks(addr, len) {
            let n = i as u8;
            let [hi, lo] = a.to_be_bytes();
            let r = self.transact(CMD_READ, &[0, hi, lo, n], |r| {
                r[2] == 0 && r[3] == hi && r[4] == lo
            })?;
            out.extend_from_slice(&r[6..6 + i]);
        }
        Ok(out)
    }

    /// Writes `data` at `addr`, then reads it back to verify.
    pub fn write_mem(&mut self, addr: u16, data: &[u8]) -> Res<()> {
        for ((n, a), part) in chunks(addr, data.len()).zip(data.chunks(CHUNK)) {
            let [hi, lo] = a.to_be_bytes();
            let mut payload = vec![0, hi, lo, n as u8];
            payload.extend_from_slice(part);
            // Match the echoed address: a late reply to the previous chunk must not confirm this one.
            self.transact(CMD_WRITE, &payload, |r| {
                r[2] == 0 && r[3] == hi && r[4] == lo
            })?;
        }
        let got = self.read_mem(addr, data.len())?;
        match got.iter().zip(data).position(|(g, w)| g != w) {
            None => Ok(()),
            Some(i) => Err(format!(
                "проверка записи не прошла: по адресу {:#05x} записано {:02x}, прочитано {:02x}",
                usize::from(addr) + i,
                data[i],
                got[i]
            )
            .into()),
        }
    }

    fn transact(&mut self, cmd: u8, payload: &[u8], ok: impl Fn(&Reply) -> bool) -> Res<Reply> {
        let give_up = Instant::now() + WAKE_WAIT;
        let mut hinted = false;
        while Instant::now() < give_up {
            self.drain()?;
            self.send(cmd, payload)?;
            let deadline = Instant::now() + TIMEOUT;
            while let Some(r) = self.recv(deadline)? {
                if r[1] == cmd && ok(&r) {
                    return Ok(r);
                }
            }
            if !hinted {
                eprintln!("мышь спит — подвигайте её…");
                hinted = true;
            }
        }
        Err("мышь не отвечает: включите её и подвигайте".into())
    }

    fn send(&self, cmd: u8, payload: &[u8]) -> io::Result<()> {
        let pkt = packet(cmd, payload);
        // HIDIOCSFEATURE(17) = _IOWR('H', 0x06, 17 bytes)
        let req = 0xC000_0000u32 | (17 << 16) | (u32::from(b'H') << 8) | 0x06;
        // SAFETY: pkt is a live 17-byte buffer, matching the length encoded in the request.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), req as _, pkt.as_ptr()) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Next valid 0x09 reply before `deadline`, or None on timeout.
    fn recv(&mut self, deadline: Instant) -> io::Result<Option<Reply>> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if !self.readable(left)? {
                return Ok(None);
            }
            let mut buf = [0u8; 64];
            let n = self.file.read(&mut buf)?;
            if n >= 17 && buf[0] == 0x09 && checksum(&buf[..17]) == 0 {
                return Ok(Some(buf[..17].try_into().unwrap_or([0; 17])));
            }
        }
    }

    /// Drops stale replies so a late answer to a previous command isn't taken as ours.
    fn drain(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 64];
        while self.readable(Duration::ZERO)? {
            let _stale = self.file.read(&mut buf)?;
        }
        Ok(())
    }

    fn readable(&self, timeout: Duration) -> io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: one valid pollfd.
        match unsafe { libc::poll(&mut pfd, 1, ms) } {
            n if n < 0 => Err(io::Error::last_os_error()),
            n => Ok(n > 0),
        }
    }
}

/// Builds the 17-byte feature report: [0x08, cmd, payload.., checksum].
fn packet(cmd: u8, payload: &[u8]) -> [u8; 17] {
    let mut b = [0u8; 17];
    b[0] = 0x08;
    b[1] = cmd;
    b[2..2 + payload.len()].copy_from_slice(payload);
    b[16] = checksum(&b[..16]);
    b
}

/// Splits [addr, addr+len) into (chunk_len, chunk_addr) pieces of at most CHUNK bytes.
fn chunks(addr: u16, len: usize) -> impl Iterator<Item = (usize, u16)> {
    (0..len)
        .step_by(CHUNK)
        .map(move |off| ((len - off).min(CHUNK), addr + off as u16))
}

fn is_receiver(uevent: &str) -> bool {
    let Some(id) = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")) else {
        return false;
    };
    let p: Vec<u32> = id
        .split(':')
        .filter_map(|s| u32::from_str_radix(s, 16).ok())
        .collect();
    matches!(p.as_slice(), [_, v, pid] if *v == VID && PIDS.contains(pid))
}

/// bcdDevice of the USB device two levels above the HID node.
fn bcd_device(hid: &Path) -> Option<u16> {
    let usb = fs::canonicalize(hid)
        .ok()?
        .parent()?
        .parent()?
        .join("bcdDevice");
    u16::from_str_radix(fs::read_to_string(usb).ok()?.trim(), 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_matches_captured_traffic() {
        // Read 10 bytes at 0x0000, as sent by the Windows tool.
        let p = packet(CMD_READ, &[0, 0, 0, 10]);
        assert_eq!(p[16], 0x55u8.wrapping_sub(8 + 8 + 10));
        assert_eq!(checksum(&p), 0);
    }

    #[test]
    fn chunking() {
        let c: Vec<_> = chunks(0x0c, 32).collect();
        assert_eq!(c, [(10, 0x0c), (10, 0x16), (10, 0x20), (2, 0x2a)]);
    }

    #[test]
    fn receiver_detection() {
        assert!(is_receiver(
            "DRIVER=hid-generic\nHID_ID=0003:000025A7:0000FA4A\n"
        ));
        assert!(!is_receiver("HID_ID=0003:0000046D:0000C52B\n"));
    }
}
