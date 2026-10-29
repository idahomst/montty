//! Observes the PTY byte stream for the OSC sequences montty cares about.
//!
//! alacritty_terminal's parser drops unknown OSC sequences, so the reader thread feeds every read
//! here as well. The scanner only observes; the bytes still reach the parser unchanged. It must
//! stay cheap: outside a sequence the work is one `memchr` for ESC.

/// Longest payload kept; anything longer is dropped and the scanner resyncs at the terminator.
pub const MAX_SEQUENCE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Payload after `7501;` (not the query), raw.
    Status(String),
    /// `OSC 7501 ; ?`
    StatusQuery,
    /// OSC 9 text or OSC 777 notify: "title: body" (or body), raw, not yet sanitised.
    Notify(String),
    /// `OSC 133 ; A` (any further params).
    PromptStart,
    /// RIS: `ESC c`.
    Reset,
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// Payload prefixes worth buffering; every other OSC is skipped without storing it.
const KEPT: [&[u8]; 4] = [b"7501;", b"9;", b"777;", b"133;"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// Saw ESC outside an OSC.
    Esc,
    /// Inside an OSC, collecting (or still deciding whether to collect) the payload.
    Osc,
    /// Inside an OSC that is not wanted or too long: wait for the terminator.
    Skip,
    /// Saw ESC inside a collecting or skipping OSC.
    OscEsc {
        collecting: bool,
    },
}

pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Signal>) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                match memchr::memchr(ESC, &bytes[i..]) {
                    Some(p) => {
                        i += p + 1;
                        self.state = State::Esc;
                        continue;
                    }
                    None => return,
                }
            }
            self.step(bytes[i], out);
            i += 1;
        }
    }

    fn step(&mut self, b: u8, out: &mut Vec<Signal>) {
        match self.state {
            State::Ground => {}
            State::Esc => self.after_esc(b, out),
            State::Osc => match b {
                BEL => self.finish(out),
                ESC => self.state = State::OscEsc { collecting: true },
                CAN | SUB => self.abort(),
                _ => {
                    self.buf.push(b);
                    self.classify();
                }
            },
            State::Skip => match b {
                BEL => self.abort(),
                ESC => self.state = State::OscEsc { collecting: false },
                CAN | SUB => self.abort(),
                _ => {}
            },
            State::OscEsc { collecting } => match b {
                b'\\' if collecting => self.finish(out),
                b'\\' => self.abort(),
                _ => {
                    // Not a string terminator: the OSC is dead and this byte follows a fresh ESC.
                    self.abort();
                    self.after_esc(b, out);
                }
            },
        }
    }

    fn after_esc(&mut self, b: u8, out: &mut Vec<Signal>) {
        self.state = match b {
            b']' => {
                self.buf.clear();
                State::Osc
            }
            b'c' => {
                out.push(Signal::Reset);
                State::Ground
            }
            ESC => State::Esc,
            _ => State::Ground,
        };
    }

    /// Decide, as payload bytes arrive, whether this OSC is kept, still undecided, or skipped.
    fn classify(&mut self) {
        let b = &self.buf[..];
        if b.len() > MAX_SEQUENCE {
            self.skip();
            return;
        }
        let mut undecided = false;
        for k in KEPT {
            if b.starts_with(k) {
                return;
            }
            if k.starts_with(b) {
                undecided = true;
            }
        }
        if !undecided {
            self.skip();
        }
    }

    fn skip(&mut self) {
        self.buf.clear();
        self.state = State::Skip;
    }

    fn abort(&mut self) {
        self.buf.clear();
        self.state = State::Ground;
    }

    fn finish(&mut self, out: &mut Vec<Signal>) {
        dispatch(&self.buf, out);
        self.abort();
    }
}

fn dispatch(payload: &[u8], out: &mut Vec<Signal>) {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    if let Some(rest) = payload.strip_prefix(b"7501;") {
        out.push(if rest == b"?" {
            Signal::StatusQuery
        } else {
            Signal::Status(text(rest))
        });
    } else if let Some(rest) = payload.strip_prefix(b"9;") {
        // ConEmu forms such as `9;4;1;50` (progress) start with a number and a `;`.
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        let conemu = digits > 0 && rest.get(digits) == Some(&b';');
        if !conemu && !rest.is_empty() {
            out.push(Signal::Notify(text(rest)));
        }
    } else if let Some(rest) = payload.strip_prefix(b"777;") {
        if let Some(rest) = rest.strip_prefix(b"notify;") {
            let (title, body) = match rest.iter().position(|&c| c == b';') {
                Some(p) => (&rest[..p], &rest[p + 1..]),
                None => (&rest[..0], rest),
            };
            if body.is_empty() {
                return;
            }
            out.push(Signal::Notify(if title.is_empty() {
                text(body)
            } else {
                format!("{}: {}", text(title), text(body))
            }));
        }
    } else if let Some(rest) = payload.strip_prefix(b"133;")
        && (rest == b"A" || rest.starts_with(b"A;"))
    {
        out.push(Signal::PromptStart);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<Signal> {
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut out);
        }
        out
    }

    fn st(s: &str) -> Signal {
        Signal::Status(s.into())
    }

    fn notify(s: &str) -> Signal {
        Signal::Notify(s.into())
    }

    #[test]
    fn status_with_st_and_bel() {
        assert_eq!(scan(&[b"\x1b]7501;state=done\x1b\\"]), [st("state=done")]);
        assert_eq!(scan(&[b"\x1b]7501;state=done\x07"]), [st("state=done")]);
    }

    #[test]
    fn query() {
        assert_eq!(scan(&[b"\x1b]7501;?\x1b\\"]), [Signal::StatusQuery]);
    }

    #[test]
    fn osc9_notification() {
        assert_eq!(
            scan(&[b"\x1b]9;Build finished\x07"]),
            [notify("Build finished")]
        );
        assert_eq!(scan(&[b"\x1b]9;4;1;50\x07"]), []);
    }

    #[test]
    fn osc777_notify() {
        assert_eq!(
            scan(&[b"\x1b]777;notify;Claude;Needs input\x07"]),
            [notify("Claude: Needs input")]
        );
        assert_eq!(
            scan(&[b"\x1b]777;notify;;Needs input\x07"]),
            [notify("Needs input")]
        );
        assert_eq!(scan(&[b"\x1b]777;other;a;b\x07"]), []);
    }

    #[test]
    fn prompt_start() {
        assert_eq!(scan(&[b"\x1b]133;A\x07"]), [Signal::PromptStart]);
        assert_eq!(scan(&[b"\x1b]133;A;cl=m\x07"]), [Signal::PromptStart]);
        assert_eq!(scan(&[b"\x1b]133;B\x07"]), []);
    }

    #[test]
    fn ris() {
        assert_eq!(scan(&[b"ab\x1bc"]), [Signal::Reset]);
    }

    #[test]
    fn other_osc_ignored() {
        assert_eq!(scan(&[b"\x1b]0;title\x07\x1b]8;;http://x\x1b\\"]), []);
    }

    #[test]
    fn split_across_feeds() {
        assert_eq!(
            scan(&[b"\x1b]75", b"01;state=blocked:kind=permission\x1b", b"\\"]),
            [st("state=blocked:kind=permission")]
        );
    }

    #[test]
    fn overlong_is_dropped_and_scanner_recovers() {
        let long = vec![b'a'; 10_000];
        assert_eq!(
            scan(&[b"\x1b]7501;state=done", &long, b"\x07\x1b]9;hi\x07"]),
            [notify("hi")]
        );
    }

    #[test]
    fn aborted_by_can_sub_or_new_esc() {
        assert_eq!(scan(&[b"\x1b]9;x\x18\x1b]9;y\x07"]), [notify("y")]);
        assert_eq!(scan(&[b"\x1b]9;x\x1a\x1b]9;y\x07"]), [notify("y")]);
        assert_eq!(scan(&[b"\x1b]9;x\x1b]9;y\x07"]), [notify("y")]);
    }

    #[test]
    fn esc_c_inside_osc_aborts_and_resets() {
        assert_eq!(scan(&[b"\x1b]9;x\x1bc"]), [Signal::Reset]);
    }

    #[test]
    #[ignore]
    fn scanner_throughput() {
        let mut chunk = vec![b'x'; 64 * 1024];
        for p in (0..chunk.len()).step_by(4096) {
            chunk[p..p + 4].copy_from_slice(b"\x1b[0m");
        }
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        let total = 256usize << 20;
        let t = std::time::Instant::now();
        for _ in 0..total / chunk.len() {
            s.feed(std::hint::black_box(&chunk), &mut out);
        }
        let rate = total as f64 / t.elapsed().as_secs_f64() / (1u64 << 30) as f64;
        eprintln!("scanner throughput: {rate:.1} GiB/s");
        assert!(out.is_empty());
        assert!(rate >= 2.0, "{rate:.1} GiB/s");
    }
}
