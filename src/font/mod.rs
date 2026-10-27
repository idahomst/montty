//! Pure-Rust font discovery (fontdb) and glyph rasterization (swash).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use alacritty_terminal::term::cell::Flags;
use swash::FontRef;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::Format;

#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size_pt: f32,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            family: "monospace".to_string(),
            size_pt: 11.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Style {
    pub fn from_flags(flags: Flags) -> Style {
        match (flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC)) {
            (false, false) => Style::Regular,
            (true, false) => Style::Bold,
            (false, true) => Style::Italic,
            (true, true) => Style::BoldItalic,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub cell_width: u32,
    pub cell_height: u32,
    pub baseline: u32,
}

/// RGBA8 glyph bitmap. `left`/`top` follow swash placement: the bitmap's
/// top-left sits at (x + left, baseline_y - top).
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
    pub data: Vec<u8>,
}

/// Loads system fonts once and shares the database.
pub fn system_db() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

fn has_family(db: &fontdb::Database, name: &str) -> Option<String> {
    db.faces()
        .flat_map(|f| f.families.iter())
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, _)| n.clone())
}

#[cfg(test)]
thread_local! {
    /// Uncached family resolutions run on this thread.
    static RESOLVE_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Maps a requested family (possibly the generic "monospace") to an installed family name.
///
/// Results (including misses) are cached per requested name for the whole process,
/// so a zoom step does not run `fc-match` again. `db` is expected to be [`system_db`].
pub fn resolve_family(requested: &str, db: &fontdb::Database) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(requested).cloned()) {
        return hit;
    }
    // Resolve without holding the lock: fc-match takes milliseconds.
    let found = resolve_uncached(requested, db);
    if let Ok(mut c) = cache.lock() {
        c.insert(requested.to_string(), found.clone());
    }
    found
}

fn resolve_uncached(requested: &str, db: &fontdb::Database) -> Option<String> {
    #[cfg(test)]
    RESOLVE_RUNS.with(|n| n.set(n.get() + 1));
    if requested.eq_ignore_ascii_case("monospace") {
        if let Ok(out) = Command::new("fc-match")
            .args(["-f", "%{family}", "monospace"])
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            let first = text.split(',').next().unwrap_or("").trim();
            if !first.is_empty()
                && let Some(n) = has_family(db, first)
            {
                return Some(n);
            }
        }
        for cand in ["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"] {
            if let Some(n) = has_family(db, cand) {
                return Some(n);
            }
        }
        return db
            .faces()
            .find(|f| f.monospaced)
            .and_then(|f| f.families.first().map(|(n, _)| n.clone()));
    }
    has_family(db, requested)
}

type Data = Arc<dyn AsRef<[u8]> + Sync + Send>;

type FileCache = Mutex<HashMap<PathBuf, Weak<dyn AsRef<[u8]> + Sync + Send>>>;

/// Process-global cache of memory-mapped font files, shared between all FontSets.
fn shared_file(path: &Path) -> Option<Data> {
    static CACHE: OnceLock<FileCache> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some(d) = cache.get(path).and_then(Weak::upgrade) {
        return Some(d);
    }
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: font files are assumed not to be truncated while montty runs
    // (same assumption fontdb makes for its own mappings).
    let map = unsafe { memmap2::Mmap::map(&file).ok()? };
    let data: Data = Arc::new(map);
    cache.insert(path.to_path_buf(), Arc::downgrade(&data));
    Some(data)
}

struct Face {
    is_color_emoji: bool,
    data: Data,
    offset: u32,
    key: swash::CacheKey,
}

impl Face {
    fn load(db: &fontdb::Database, id: fontdb::ID) -> Option<Face> {
        let (source, index) = db.face_source(id)?;
        let data: Data = match source {
            fontdb::Source::Binary(d) => d,
            fontdb::Source::SharedFile(_, d) => d,
            fontdb::Source::File(path) => shared_file(&path)?,
        };
        let font = FontRef::from_index((*data).as_ref(), index as usize)?;
        let (offset, key) = (font.offset, font.key);
        let is_color_emoji = db
            .face(id)
            .is_some_and(|f| f.families.iter().any(|(n, _)| n == "Noto Color Emoji"));
        Some(Face {
            is_color_emoji,
            data,
            offset,
            key,
        })
    }

    /// Reuses the cache key from load time so swash's scaler cache can hit.
    fn font(&self) -> FontRef<'_> {
        FontRef {
            data: (*self.data).as_ref(),
            offset: self.offset,
            key: self.key,
        }
    }
}

pub struct FontSet {
    /// Unique per loaded set in the process; the glyph atlas keys by it.
    key: u32,
    db: Arc<fontdb::Database>,
    px: f32,
    metrics: Metrics,
    /// Indexed by `Style`; `None` entries fall back to Regular.
    primary: [Option<Face>; 4],
    fallback_faces: Vec<Face>,
    fallback_by_id: HashMap<fontdb::ID, usize>,
    /// Result of fallback search per char, including failed searches.
    fallback: HashMap<char, Option<usize>>,
    ctx: ScaleContext,
    db_scans: usize,
}

fn is_emoji_cp(ch: char) -> bool {
    matches!(ch as u32,
        0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF)
}

impl FontSet {
    pub fn load(spec: &FontSpec, scale: f64, db: Arc<fontdb::Database>) -> Result<FontSet, String> {
        let family = resolve_family(&spec.family, &db)
            .ok_or_else(|| format!("no font found for family '{}'", spec.family))?;
        let px = (spec.size_pt as f64 * 96.0 / 72.0 * scale) as f32;

        let mut primary: [Option<Face>; 4] = [None, None, None, None];
        let mut regular_id = None;
        for style in [
            Style::Regular,
            Style::Bold,
            Style::Italic,
            Style::BoldItalic,
        ] {
            let (weight, fstyle) = match style {
                Style::Regular => (fontdb::Weight::NORMAL, fontdb::Style::Normal),
                Style::Bold => (fontdb::Weight::BOLD, fontdb::Style::Normal),
                Style::Italic => (fontdb::Weight::NORMAL, fontdb::Style::Italic),
                Style::BoldItalic => (fontdb::Weight::BOLD, fontdb::Style::Italic),
            };
            let q = fontdb::Query {
                families: &[fontdb::Family::Name(&family)],
                weight,
                stretch: fontdb::Stretch::Normal,
                style: fstyle,
            };
            if let Some(id) = db.query(&q) {
                // Slots resolving to the same face as Regular stay empty (fall back to Regular).
                if style == Style::Regular {
                    regular_id = Some(id);
                } else if Some(id) == regular_id {
                    continue;
                }
                primary[style.index()] = Face::load(&db, id);
            }
        }
        let regular = primary[0]
            .as_ref()
            .ok_or_else(|| format!("cannot load font '{family}'"))?;
        let font = regular.font();
        let m = font.metrics(&[]).scale(px);
        let advance = font
            .glyph_metrics(&[])
            .scale(px)
            .advance_width(font.charmap().map('M'));
        let metrics = Metrics {
            cell_width: (advance.round() as u32).max(1),
            cell_height: ((m.ascent + m.descent + m.leading).ceil() as u32).max(1),
            baseline: m.ascent.round() as u32,
        };
        static NEXT_KEY: AtomicU32 = AtomicU32::new(1);
        Ok(FontSet {
            key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            db,
            px,
            metrics,
            primary,
            fallback_faces: Vec::new(),
            fallback_by_id: HashMap::new(),
            fallback: HashMap::new(),
            ctx: ScaleContext::new(),
            db_scans: 0,
        })
    }

    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// Identifies this set (family, size and scale as loaded) for the process's lifetime.
    pub fn key(&self) -> u32 {
        self.key
    }

    #[cfg(test)]
    fn db_scans(&self) -> usize {
        self.db_scans
    }

    #[cfg(test)]
    fn regular_data_ptr(&self) -> *const u8 {
        (*self.primary[0].as_ref().unwrap().data).as_ref().as_ptr()
    }

    /// True if the fallback search for `ch` has already been done (hit or miss).
    pub fn is_cached(&self, ch: char) -> bool {
        self.fallback.contains_key(&ch)
    }

    fn find_fallback(&mut self, ch: char) -> Option<usize> {
        if let Some(r) = self.fallback.get(&ch) {
            return *r;
        }
        // Already-loaded fallback faces first: keeps scripts on one font and avoids db scans.
        // Emoji codepoints only match loaded color-emoji faces, so a generic fallback
        // face loaded earlier cannot steal them.
        let emoji = is_emoji_cp(ch);
        let loaded = self
            .fallback_faces
            .iter()
            .position(|f| (!emoji || f.is_color_emoji) && f.font().charmap().map(ch) != 0);
        let idx = loaded.or_else(|| {
            let id = self.scan_db(ch)?;
            if let Some(&i) = self.fallback_by_id.get(&id) {
                return Some(i);
            }
            let face = Face::load(&self.db, id)?;
            self.fallback_faces.push(face);
            let i = self.fallback_faces.len() - 1;
            self.fallback_by_id.insert(id, i);
            Some(i)
        });
        self.fallback.insert(ch, idx);
        idx
    }

    /// Single pass over the db for a face covering `ch`. Prefers upright regular weight,
    /// monospaced, then non-serif; stops early on a perfect match.
    fn scan_db(&mut self, ch: char) -> Option<fontdb::ID> {
        self.db_scans += 1;
        let emoji = is_emoji_cp(ch);
        let mut best: Option<(u32, fontdb::ID)> = None;
        for f in self.db.faces() {
            let is_emoji_face = f.families.iter().any(|(n, _)| n == "Noto Color Emoji");
            if emoji && !is_emoji_face && best.is_some_and(|(s, _)| s >= 100) {
                continue;
            }
            let covered = self
                .db
                .with_face_data(f.id, |d, i| {
                    FontRef::from_index(d, i as usize).is_some_and(|f| f.charmap().map(ch) != 0)
                })
                .unwrap_or(false);
            if !covered {
                continue;
            }
            let mut score = 1;
            if emoji && is_emoji_face {
                score += 100;
            }
            if f.style == fontdb::Style::Normal && f.weight == fontdb::Weight::NORMAL {
                score += 4;
            }
            if f.monospaced {
                score += 2;
            }
            if !f
                .families
                .iter()
                .any(|(n, _)| n.to_ascii_lowercase().contains("serif"))
                || f.families
                    .iter()
                    .any(|(n, _)| n.to_ascii_lowercase().contains("sans"))
            {
                score += 1;
            }
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, f.id));
            }
            if (!emoji && score >= 8) || (emoji && is_emoji_face) {
                break;
            }
        }
        best.map(|(_, id)| id)
    }

    pub fn rasterize(&mut self, ch: char, style: Style) -> Option<RasterGlyph> {
        let primary = self.primary[style.index()]
            .as_ref()
            .or(self.primary[0].as_ref());
        let in_primary = primary
            .map(|f| f.font())
            .is_some_and(|f| f.charmap().map(ch) != 0);
        // Which face to render with: (is_fallback, index)
        let fallback_idx = if in_primary {
            None
        } else {
            Some(self.find_fallback(ch)?)
        };

        let face: &Face = match fallback_idx {
            Some(i) => &self.fallback_faces[i],
            None => self.primary[style.index()]
                .as_ref()
                .or(self.primary[0].as_ref())?,
        };
        let font = face.font();
        let gid = font.charmap().map(ch);
        if gid == 0 {
            return None;
        }
        let mut scaler = self.ctx.builder(font).size(self.px).hint(true).build();
        let image = Render::new(&[
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::ColorOutline(0),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .render(&mut scaler, gid)?;
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            return None;
        }
        match image.content {
            Content::Mask => {
                let mut data = Vec::with_capacity(image.data.len() * 4);
                for &a in &image.data {
                    data.extend_from_slice(&[255, 255, 255, a]);
                }
                Some(RasterGlyph {
                    width: p.width,
                    height: p.height,
                    left: p.left,
                    top: p.top,
                    color: false,
                    data,
                })
            }
            Content::Color => {
                let mut data = image.data;
                // swash composites COLR layers premultiplied; bitmaps (CBDT, sbix) are
                // straight already. The atlas and the cell shader expect straight alpha.
                if matches!(image.source, Source::ColorOutline(_)) {
                    unpremultiply(&mut data);
                }
                let g = RasterGlyph {
                    width: p.width,
                    height: p.height,
                    left: p.left,
                    top: p.top,
                    color: true,
                    data,
                };
                Some(fit_color(g, self.metrics.cell_height))
            }
            Content::SubpixelMask => None,
        }
    }
}

/// A font size in 1/100 pt and the scale's bits.
type SizeKey = (u32, u64);

/// Font sets by (family, size, scale), loaded on first use and kept: panes with different
/// profiles or zoom share one set per distinct font. Nothing is evicted.
pub struct FontCache {
    db: Arc<fontdb::Database>,
    sets: HashMap<u32, FontSet>,
    /// family → (size in 1/100 pt, scale bits) → key, or the load error. Two levels, so a
    /// lookup by `&str` allocates nothing; a shared error neither (a pane whose family is
    /// missing asks every frame).
    index: HashMap<String, HashMap<SizeKey, Result<u32, Arc<str>>>>,
    #[cfg(test)]
    loads: usize,
}

impl FontCache {
    pub fn new(db: Arc<fontdb::Database>) -> FontCache {
        FontCache {
            db,
            sets: HashMap::new(),
            index: HashMap::new(),
            #[cfg(test)]
            loads: 0,
        }
    }

    /// The set for `family` at `size_pt` and `scale`, loaded on the first request. A
    /// family without an installed font is `Err("no font found for family '<f>'")`; errors
    /// are remembered too, so asking again costs a lookup.
    pub fn get(
        &mut self,
        family: &str,
        size_pt: f32,
        scale: f64,
    ) -> Result<&mut FontSet, Arc<str>> {
        let size = ((size_pt * 100.0).round() as u32, scale.to_bits());
        let known = self.index.get(family).and_then(|m| m.get(&size));
        let key = match known {
            Some(k) => k.clone()?,
            None => {
                let spec = FontSpec {
                    family: family.to_string(),
                    size_pt,
                };
                #[cfg(test)]
                {
                    self.loads += 1;
                }
                let loaded = FontSet::load(&spec, scale, self.db.clone())
                    .map(|set| {
                        let key = set.key();
                        self.sets.insert(key, set);
                        key
                    })
                    .map_err(Arc::from);
                self.index
                    .entry(family.to_string())
                    .or_default()
                    .insert(size, loaded.clone());
                loaded?
            }
        };
        self.sets
            .get_mut(&key)
            .ok_or_else(|| format!("font set {key} missing from the cache").into())
    }

    /// Drops every set whose key is not in `live`, and the index entries pointing at it
    /// (remembered load errors stay). Returns how many sets were dropped. Keys are never
    /// reused, so a dropped key can't alias a later set.
    pub fn retain(&mut self, live: &HashSet<u32>) -> usize {
        let before = self.sets.len();
        self.sets.retain(|k, _| live.contains(k));
        for sizes in self.index.values_mut() {
            sizes.retain(|_, r| r.as_ref().map_or(true, |k| self.sets.contains_key(k)));
        }
        self.index.retain(|_, sizes| !sizes.is_empty());
        before - self.sets.len()
    }

    /// A set already loaded by `get`.
    pub fn by_key(&mut self, key: u32) -> Option<&mut FontSet> {
        self.sets.get_mut(&key)
    }
}

/// Premultiplied RGBA to straight alpha in place: rgb·255/a, rounded and clamped;
/// fully transparent pixels are left alone.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// Scales a color bitmap down so its height does not exceed `target_h`, keeping aspect.
fn fit_color(g: RasterGlyph, target_h: u32) -> RasterGlyph {
    if g.height <= target_h || target_h == 0 {
        return g;
    }
    let s = target_h as f32 / g.height as f32;
    let nw = ((g.width as f32 * s).round() as u32).max(1);
    let nh = target_h;
    let mut data = vec![0u8; (nw * nh * 4) as usize];
    // Box filter over premultiplied alpha.
    for y in 0..nh {
        let y0 = (y as f32 / s) as u32;
        let y1 = (((y + 1) as f32 / s).ceil() as u32)
            .min(g.height)
            .max(y0 + 1);
        for x in 0..nw {
            let x0 = (x as f32 / s) as u32;
            let x1 = (((x + 1) as f32 / s).ceil() as u32)
                .min(g.width)
                .max(x0 + 1);
            let (mut r, mut gg, mut b, mut a, mut n) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for sy in y0..y1.min(g.height) {
                for sx in x0..x1.min(g.width) {
                    let i = ((sy * g.width + sx) * 4) as usize;
                    let al = g.data[i + 3] as f32 / 255.0;
                    r += g.data[i] as f32 * al;
                    gg += g.data[i + 1] as f32 * al;
                    b += g.data[i + 2] as f32 * al;
                    a += al;
                    n += 1.0;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            if a > 0.0 && n > 0.0 {
                data[o] = (r / a).round().min(255.0) as u8;
                data[o + 1] = (gg / a).round().min(255.0) as u8;
                data[o + 2] = (b / a).round().min(255.0) as u8;
                data[o + 3] = (a / n * 255.0).round().min(255.0) as u8;
            }
        }
    }
    RasterGlyph {
        width: nw,
        height: nh,
        left: (g.left as f32 * s).round() as i32,
        top: (g.top as f32 * s).round() as i32,
        color: true,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(scale: f64) -> FontSet {
        FontSet::load(&FontSpec::default(), scale, system_db()).expect("load default font")
    }

    #[test]
    fn unpremultiply_rounds_and_skips_transparent() {
        let mut px = [
            100, 50, 0, 128, // half covered: rgb·255/a, rounded
            7, 7, 7, 0, // a = 0: left alone
            10, 20, 30, 255, // opaque: unchanged
            200, 0, 0, 100, // rgb > a (invalid): clamped
        ];
        unpremultiply(&mut px);
        assert_eq!(
            px,
            [
                199, 100, 0, 128, 7, 7, 7, 0, 10, 20, 30, 255, 255, 0, 0, 100
            ]
        );
    }

    #[test]
    fn resolves_monospace() {
        assert!(resolve_family("monospace", &system_db()).is_some());
    }

    #[test]
    fn resolve_family_is_cached() {
        let db = system_db();
        let runs = || RESOLVE_RUNS.with(std::cell::Cell::get);
        // A name no other test uses: the first call resolves, the second hits the cache.
        let before = runs();
        let first = resolve_family("Montty Cache Probe", &db);
        assert_eq!(first, None);
        assert_eq!(runs(), before + 1);
        assert_eq!(resolve_family("Montty Cache Probe", &db), first);
        assert_eq!(runs(), before + 1, "second lookup ran the resolution again");
        // "monospace" (fc-match) likewise runs at most once per process.
        let mono = resolve_family("monospace", &db);
        let after_first = runs();
        assert_eq!(resolve_family("monospace", &db), mono);
        assert_eq!(runs(), after_first);
    }

    #[test]
    fn metrics_sane() {
        let m = set(1.0).metrics();
        assert!((5..=12).contains(&m.cell_width), "{m:?}");
        assert!(m.cell_height > m.cell_width, "{m:?}");
        assert!(m.baseline < m.cell_height, "{m:?}");
    }

    #[test]
    fn scale_doubles_metrics() {
        let a = set(1.0).metrics().cell_height as i64;
        let b = set(2.0).metrics().cell_height as i64;
        assert!((b - 2 * a).abs() <= 1, "{a} {b}");
    }

    #[test]
    fn rasterizes_ascii() {
        let g = set(1.0).rasterize('A', Style::Regular).expect("glyph");
        assert!(g.width > 0 && !g.color);
        assert_eq!(g.data.len() as u32, g.width * g.height * 4);
    }

    #[test]
    fn space_is_none() {
        assert!(set(1.0).rasterize(' ', Style::Regular).is_none());
    }

    #[test]
    fn falls_back_for_cjk() {
        let db = system_db();
        let covers = db.faces().any(|f| {
            db.with_face_data(f.id, |d, i| {
                FontRef::from_index(d, i as usize).is_some_and(|f| f.charmap().map('中') != 0)
            })
            .unwrap_or(false)
        });
        if !covers {
            return;
        }
        assert!(set(1.0).rasterize('中', Style::Regular).is_some());
    }

    #[test]
    fn missing_glyph_is_cached() {
        let mut s = set(1.0);
        assert!(!s.is_cached('\u{10FFFD}'));
        assert!(s.rasterize('\u{10FFFD}', Style::Regular).is_none());
        assert!(s.is_cached('\u{10FFFD}'));
    }

    #[test]
    fn font_sets_share_file_data() {
        let (a, b) = (set(1.0), set(1.0));
        assert_eq!(a.regular_data_ptr(), b.regular_data_ptr());
    }

    #[test]
    fn second_cjk_char_needs_no_db_scan() {
        let db = system_db();
        let covers = |c: char| {
            db.faces().any(|f| {
                db.with_face_data(f.id, |d, i| {
                    FontRef::from_index(d, i as usize).is_some_and(|f| f.charmap().map(c) != 0)
                })
                .unwrap_or(false)
            })
        };
        if !covers('中') || !covers('文') {
            return;
        }
        let mut s = set(1.0);
        assert!(s.rasterize('中', Style::Regular).is_some());
        let scans = s.db_scans();
        assert!(scans >= 1, "first fallback must scan the db");
        assert!(s.rasterize('文', Style::Regular).is_some());
        assert_eq!(
            s.db_scans(),
            scans,
            "second char should use the loaded fallback face"
        );
    }

    #[test]
    fn emoji_stays_color_after_other_fallbacks() {
        let mut s = set(1.0);
        if s.rasterize('\u{1F600}', Style::Regular)
            .is_none_or(|g| !g.color)
        {
            return; // no color emoji font installed
        }
        let mut s = set(1.0);
        for c in ['中', 'あ', 'ℵ', '⌬'] {
            s.rasterize(c, Style::Regular);
        }
        assert!(s.rasterize('\u{1F600}', Style::Regular).unwrap().color);
    }

    #[test]
    fn style_from_flags() {
        assert_eq!(
            Style::from_flags(Flags::BOLD | Flags::ITALIC),
            Style::BoldItalic
        );
        assert_eq!(Style::from_flags(Flags::empty()), Style::Regular);
    }

    #[test]
    fn bold_differs_and_emoji_is_color_and_fits() {
        let mut s = set(1.0);
        let reg = s.rasterize('A', Style::Regular).unwrap();
        let bold = s.rasterize('A', Style::Bold).unwrap();
        assert_ne!(reg.data, bold.data);
        if let Some(e) = s.rasterize('😀', Style::Regular) {
            assert!(e.color);
            assert!(e.height <= s.metrics().cell_height);
        }
    }

    #[test]
    fn font_cache_reuses_and_keys_differ() {
        let mut c = FontCache::new(system_db());
        let a = c.get("monospace", 11.0, 1.0).expect("monospace").key();
        assert_eq!(c.get("monospace", 11.0, 1.0).unwrap().key(), a);
        assert_eq!(c.loads, 1, "the same request loads once");
        let b = c.get("monospace", 12.0, 1.0).unwrap().key();
        assert_ne!(a, b);
        assert_ne!(c.get("monospace", 11.0, 2.0).unwrap().key(), a);
        let e = c
            .get("NoSuchFont", 11.0, 1.0)
            .err()
            .expect("unknown family");
        assert!(e.contains("NoSuchFont"), "{e}");
        // Asked again (every frame for a pane using it): the remembered error, not a copy.
        let again = c.get("NoSuchFont", 11.0, 1.0).err().unwrap();
        assert!(Arc::ptr_eq(&e, &again));
        assert_eq!(c.loads, 4);
    }

    #[test]
    fn retain_keeps_live_and_drops_the_rest() {
        let mut c = FontCache::new(system_db());
        let a = c.get("monospace", 11.0, 1.0).unwrap().key();
        let b = c.get("monospace", 14.0, 1.0).unwrap().key();
        assert_eq!(c.loads, 2);
        assert_eq!(c.retain(&HashSet::from([a])), 1);
        assert!(c.by_key(b).is_none());
        assert_eq!(c.get("monospace", 11.0, 1.0).unwrap().key(), a);
        assert_eq!(c.loads, 2, "the kept set is not reloaded");
        let b2 = c.get("monospace", 14.0, 1.0).unwrap().key();
        assert_eq!(c.loads, 3, "the dropped set reloads");
        assert_ne!(b2, b, "keys are never reused");
    }

    #[test]
    fn retain_keeps_errors() {
        let mut c = FontCache::new(system_db());
        let e = c.get("NoSuchFont", 11.0, 1.0).err().unwrap();
        assert_eq!(c.retain(&HashSet::new()), 0);
        let again = c.get("NoSuchFont", 11.0, 1.0).err().unwrap();
        assert!(Arc::ptr_eq(&e, &again));
        assert_eq!(c.loads, 1);
    }

    #[test]
    fn font_set_is_send() {
        fn is_send<T: Send>() {}
        is_send::<FontSet>();
    }
}
