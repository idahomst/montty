//! Background image decoding in a helper process (`montty --decode-image`): release builds
//! abort on panic, so a decoder panic, allocation failure or hang must only kill a child.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::render::image::{Rgba8, decode, downscale};

/// Image files larger than this aren't read.
pub(crate) const MAX_FILE: u64 = 600 << 20;

/// The bytes of the file at `path`, unless it is larger than `MAX_FILE` (checked before
/// reading, and while reading in case it grows).
pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    const TOO_LARGE: &str = "image file too large";
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err(TOO_LARGE.into());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    f.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(TOO_LARGE.into());
    }
    Ok(bytes)
}

/// The helper's address space limit: a decoder bomb fails as an allocation error here,
/// inside the child.
const HELPER_AS_LIMIT: u64 = 2 << 30;

/// Error text lines longer than this are cut.
const MAX_ERR: usize = 200;

/// Runs the helper for `args` (after `--decode-image`: `<path> <max_w> <max_h>`): decodes the
/// file, downscales it to within the maximum and writes `w` and `h` (u32 LE) and the RGBA
/// bytes to stdout. Returns the exit code: 0, or 2 after writing the error to stderr.
pub fn run_helper(args: &[String]) -> i32 {
    match helper(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

fn helper(args: &[String]) -> Result<(), String> {
    let [path, w, h] = args else {
        return Err("usage: --decode-image <path> <max_w> <max_h>".into());
    };
    let dim = |s: &String| s.parse::<u32>().ok().filter(|&n| n > 0);
    let (Some(w), Some(h)) = (dim(w), dim(h)) else {
        return Err("bad size limit".into());
    };
    no_core_dumps();
    limit_address_space();
    let bytes = read_capped(Path::new(path))?;
    let img = downscale(decode(&bytes)?, w, h);
    let mut out = std::io::stdout().lock();
    let mut write = || -> std::io::Result<()> {
        out.write_all(&img.w.to_le_bytes())?;
        out.write_all(&img.h.to_le_bytes())?;
        out.write_all(&img.px)?;
        out.flush()
    };
    write().map_err(|e| e.to_string())
}

/// No core dump when the decoder dies of a signal: it holds only a decoded image.
fn no_core_dumps() {
    let lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit only reads the struct.
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &lim) };
}

fn limit_address_space() {
    let lim = libc::rlimit {
        rlim_cur: HELPER_AS_LIMIT as libc::rlim_t,
        rlim_max: HELPER_AS_LIMIT as libc::rlim_t,
    };
    // Best effort: without the limit the decode still works, just unbounded.
    // SAFETY: setrlimit only reads the struct.
    unsafe { libc::setrlimit(libc::RLIMIT_AS, &lim) };
}

/// Kills and reaps the child on drop, whatever way the parent side ends.
struct Reaper(Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads `r` to the end, but at most `cap` bytes, on a new thread; the result is sent on the
/// returned channel.
fn read_in_thread(mut r: impl Read + Send + 'static, cap: u64) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut r).take(cap).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Decodes `file` in a helper process started from `exe` (`--decode-image <file> <max>`),
/// within `timeout`: the image, or why not. Whatever the helper does (crash, hang, send
/// garbage) only fails this call.
pub fn decode_in_helper(
    exe: &Path,
    file: &Path,
    max: (u32, u32),
    timeout: Duration,
) -> Result<Rgba8, String> {
    const CRASHED: &str = "decoder crashed";
    const BAD: &str = "decoder sent a bad image";
    const TIMED_OUT: &str = "decoder timed out";
    let deadline = Instant::now() + timeout;
    let (mw, mh) = (u64::from(max.0), u64::from(max.1));
    // One byte more than can be valid, to tell "too much" from "just enough".
    let cap = 8 + mw * mh * 4;
    let mut child = Command::new(exe)
        .arg("--decode-image")
        .arg(file)
        .arg(max.0.to_string())
        .arg(max.1.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't start the decoder: {e}"))?;
    let out = child.stdout.take().map(|o| read_in_thread(o, cap + 1));
    let err = child.stderr.take().map(|e| read_in_thread(e, 4096));
    let mut child = Reaper(child);
    let left = |d: Instant| d.saturating_duration_since(Instant::now());
    let bytes = match out {
        Some(rx) => rx.recv_timeout(left(deadline)).map_err(|_| TIMED_OUT)?,
        None => Vec::new(),
    };
    if bytes.len() as u64 > cap {
        return Err(BAD.into());
    }
    let status = loop {
        match child.0.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => return Err(TIMED_OUT.into()),
            Err(_) => return Err(CRASHED.into()),
        }
    };
    match status.code() {
        Some(0) => parse(&bytes, max).ok_or_else(|| BAD.to_string()),
        Some(2) => {
            let text = err
                .and_then(|rx| rx.recv_timeout(Duration::from_millis(200)).ok())
                .unwrap_or_default();
            let text = String::from_utf8_lossy(&text);
            let line = text.lines().next().unwrap_or("").trim();
            match line.is_empty() {
                true => Err(CRASHED.into()),
                false => Err(line.chars().take(MAX_ERR).collect()),
            }
        }
        _ => Err(CRASHED.into()),
    }
}

/// The helper's output: `w`, `h` (u32 LE) and exactly `w*h*4` bytes, within `max`.
fn parse(bytes: &[u8], max: (u32, u32)) -> Option<Rgba8> {
    let (head, px) = bytes.split_at_checked(8)?;
    let w = u32::from_le_bytes(head[..4].try_into().ok()?);
    let h = u32::from_le_bytes(head[4..].try_into().ok()?);
    if w == 0 || h == 0 || w > max.0 || h > max.1 {
        return None;
    }
    let len = usize::try_from(u64::from(w) * u64::from(h) * 4).ok()?;
    (px.len() == len).then(|| Rgba8 {
        w,
        h,
        px: px.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_files_are_not_read() {
        let dir = std::env::temp_dir().join(format!("montty-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("huge.png");
        // Sparse: no disk space or reading involved.
        std::fs::File::create(&f)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        let e = read_capped(&f).unwrap_err();
        std::fs::write(&f, b"small").unwrap();
        let ok = read_capped(&f);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(e, "image file too large");
        assert_eq!(ok.unwrap(), b"small");
    }
}
