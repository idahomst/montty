//! The image decoder helper (`montty --decode-image`) and its parent side.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use montty::decoder::decode_in_helper;
use montty::render::image::Rgba8;

const EXE: &str = env!("CARGO_BIN_EXE_montty");

fn tmp(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("montty-dec-{}-{n}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().unwrap();
    let px: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).flat_map(|x| f(x, y)).collect::<Vec<_>>())
        .collect();
    wr.write_image_data(&px).unwrap();
    drop(wr);
    out
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(EXE).args(args).output().unwrap()
}

#[test]
fn helper_decodes_a_png() {
    let d = tmp("ok");
    let f = d.join("a.png");
    std::fs::write(&f, png(3, 2, |x, y| [x as u8 * 10, y as u8 * 20, 7, 255])).unwrap();
    let o = run(&["--decode-image", f.to_str().unwrap(), "100", "100"]);
    assert_eq!(o.status.code(), Some(0));
    let b = &o.stdout;
    assert_eq!(&b[..4], &3u32.to_le_bytes());
    assert_eq!(&b[4..8], &2u32.to_le_bytes());
    assert_eq!(b.len(), 8 + 3 * 2 * 4);
    assert_eq!(&b[8..12], &[0, 0, 7, 255]);
    assert_eq!(&b[8 + 4 * 4..8 + 4 * 5], &[10, 20, 7, 255]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn helper_downscales_to_the_limit() {
    let d = tmp("down");
    let f = d.join("a.png");
    std::fs::write(&f, png(40, 20, |_, _| [1, 2, 3, 255])).unwrap();
    let img = decode_in_helper(Path::new(EXE), &f, (10, 10), Duration::from_secs(10)).unwrap();
    assert!(img.w <= 10 && img.h <= 10 && img.w > 0 && img.h > 0);
    assert_eq!(img.px.len(), (img.w * img.h * 4) as usize);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn helper_reports_a_missing_file() {
    let d = tmp("missing");
    let f = d.join("nope.png");
    let o = run(&["--decode-image", f.to_str().unwrap(), "100", "100"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    let want = std::fs::File::open(&f).unwrap_err().to_string();
    assert_eq!(String::from_utf8_lossy(&o.stderr).trim(), want);
    let e = decode_in_helper(Path::new(EXE), &f, (100, 100), Duration::from_secs(10));
    assert_eq!(e.unwrap_err(), want);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn helper_refuses_a_forged_header() {
    let d = tmp("forged");
    let f = d.join("bomb.png");
    // A valid-looking IHDR declaring 100000×100000; the CRC is wrong, but the size is
    // checked from the header before anything reads on.
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&100_000u32.to_be_bytes());
    b.extend_from_slice(&100_000u32.to_be_bytes());
    b.extend_from_slice(&[8, 2, 0, 0, 0]);
    b.extend_from_slice(&[0; 4]);
    std::fs::write(&f, b).unwrap();
    let e = decode_in_helper(Path::new(EXE), &f, (100, 100), Duration::from_secs(10));
    let e = e.unwrap_err();
    assert!(
        e.contains("too large") || e.contains("CRC") || e.contains("crc"),
        "{e}"
    );
    assert_ne!(e, "decoder crashed");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn helper_rejects_bad_format() {
    let d = tmp("bad");
    let f = d.join("x.png");
    std::fs::write(&f, b"GIF89a....").unwrap();
    let e = decode_in_helper(Path::new(EXE), &f, (100, 100), Duration::from_secs(10));
    assert_eq!(e.unwrap_err(), "unsupported image format");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn helper_decodes_in_a_child_png_roundtrip() {
    let d = tmp("rt");
    let f = d.join("a.png");
    std::fs::write(&f, png(5, 4, |x, y| [x as u8, y as u8, 9, 200])).unwrap();
    let img = decode_in_helper(Path::new(EXE), &f, (100, 100), Duration::from_secs(10)).unwrap();
    assert_eq!((img.w, img.h), (5, 4));
    assert_eq!(&img.px[..4], &[0, 0, 9, 200]);
    let _ = std::fs::remove_dir_all(d);
}

/// `decode_in_helper`, tried again while another test's fork still holds the script's
/// write fd (ETXTBSY).
fn decode_retrying(exe: &Path, max: (u32, u32), timeout: Duration) -> Result<Rgba8, String> {
    let mut r = decode_in_helper(exe, Path::new("/x.png"), max, timeout);
    for _ in 0..50 {
        match &r {
            Err(e) if e.contains("busy") => std::thread::sleep(Duration::from_millis(20)),
            _ => break,
        }
        r = decode_in_helper(exe, Path::new("/x.png"), max, timeout);
    }
    r
}

/// A fake helper: a shell script that ignores its arguments.
fn fake(d: &Path, body: &str) -> PathBuf {
    let p = d.join("fake.sh");
    std::fs::write(&p, format!("#!/bin/sh\nulimit -c 0\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn via(body: &str, max: (u32, u32), timeout: Duration) -> Result<Rgba8, String> {
    let d = tmp("fake");
    let exe = fake(&d, body);
    let r = decode_retrying(&exe, max, timeout);
    let _ = std::fs::remove_dir_all(d);
    r
}

const T: Duration = Duration::from_secs(10);

#[test]
fn fake_segv_and_abort_are_crashes() {
    assert_eq!(
        via("kill -SEGV $$", (9, 9), T).unwrap_err(),
        "decoder crashed"
    );
    assert_eq!(
        via("kill -ABRT $$", (9, 9), T).unwrap_err(),
        "decoder crashed"
    );
    assert_eq!(via("exit 1", (9, 9), T).unwrap_err(), "decoder crashed");
    assert_eq!(via("exit 101", (9, 9), T).unwrap_err(), "decoder crashed");
}

#[test]
fn fake_timeout_kills_the_child() {
    let d = tmp("sleep");
    let exe = fake(&d, &format!("echo $$ > {}/pid\nsleep 30", d.display()));
    let t = Instant::now();
    let r = decode_retrying(&exe, (9, 9), Duration::from_secs(2));
    assert_eq!(r.unwrap_err(), "decoder timed out");
    // Gave up at the timeout, not after the script's 30 s sleep.
    assert!(t.elapsed() >= Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    let pid = std::fs::read_to_string(d.join("pid")).unwrap();
    let alive = Path::new(&format!("/proc/{}", pid.trim())).exists();
    assert!(!alive, "child still there");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn fake_short_or_oversized_payload_is_bad() {
    // 4×4 claimed, 10 bytes sent.
    let short = r#"printf '\004\000\000\000\004\000\000\000'; printf '0123456789'"#;
    assert_eq!(
        via(short, (9, 9), T).unwrap_err(),
        "decoder sent a bad image"
    );
    // 4×4 with max 2×2.
    let big = r#"printf '\004\000\000\000\004\000\000\000'; head -c 64 /dev/zero"#;
    assert_eq!(via(big, (2, 2), T).unwrap_err(), "decoder sent a bad image");
    // Exactly right is accepted.
    let img = via(big, (9, 9), T).unwrap();
    assert_eq!((img.w, img.h, img.px.len()), (4, 4, 64));
    // Trailing bytes are bad too; zero size is bad.
    let extra = r#"printf '\004\000\000\000\004\000\000\000'; head -c 65 /dev/zero"#;
    assert_eq!(
        via(extra, (9, 9), T).unwrap_err(),
        "decoder sent a bad image"
    );
    let zero = r#"printf '\000\000\000\000\004\000\000\000'"#;
    assert_eq!(
        via(zero, (9, 9), T).unwrap_err(),
        "decoder sent a bad image"
    );
    assert_eq!(
        via("true", (9, 9), T).unwrap_err(),
        "decoder sent a bad image"
    );
}

#[test]
fn fake_exit_2_passes_stderr() {
    let r = via(
        "echo 'bad thing happened' >&2; echo second >&2; exit 2",
        (9, 9),
        T,
    );
    assert_eq!(r.unwrap_err(), "bad thing happened");
    let long = "printf '%0300d\\n' 7 >&2; exit 2";
    assert_eq!(via(long, (9, 9), T).unwrap_err().chars().count(), 200);
}

#[test]
fn spawn_failure_is_an_error() {
    let e = decode_in_helper(Path::new("/nonexistent/montty"), Path::new("/x"), (9, 9), T);
    assert!(e.is_err());
}
