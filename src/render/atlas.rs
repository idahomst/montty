//! Shelf-packed RGBA glyph atlas. GPU upload goes through `AtlasTexture`.

use std::collections::HashMap;

use crate::font::{FontSet, Style};

/// Shelf packer with 1px padding between rectangles.
pub struct ShelfPacker {
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    row_h: u32,
}

impl ShelfPacker {
    pub fn new(w: u32, h: u32) -> Self {
        ShelfPacker {
            w,
            h,
            x: 0,
            y: 0,
            row_h: 0,
        }
    }

    pub fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.w || h > self.h {
            return None;
        }
        let (mut x, mut y, mut row_h) = (self.x, self.y, self.row_h);
        if x + w > self.w {
            y += row_h + 1;
            x = 0;
            row_h = 0;
        }
        if y + h > self.h {
            return None;
        }
        self.x = x + w + 1;
        self.y = y;
        self.row_h = row_h.max(h);
        Some((x, y))
    }

    /// Whether a w x h rectangle could ever fit (i.e. into an empty packer).
    pub fn fits_empty(&self, w: u32, h: u32) -> bool {
        w <= self.w && h <= self.h
    }

    pub fn clear(&mut self) {
        self.x = 0;
        self.y = 0;
        self.row_h = 0;
    }
}

pub trait AtlasTexture {
    fn upload(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]);
    /// Reallocate the texture as `size` x `size`; its contents are undefined afterwards.
    /// False if the allocation failed (out of memory); the texture then has no storage.
    fn resize(&mut self, size: u32) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasGlyph {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub left: i16,
    pub top: i16,
    pub color: bool,
}

pub struct GlyphAtlas {
    packer: ShelfPacker,
    /// By (`FontSet::key`, char, style): fonts of different panes share the texture.
    cache: HashMap<(u32, char, Style), Option<AtlasGlyph>>,
    generation: u64,
    size: u32,
    max: u32,
    /// The atlas already filled (and cleared) this frame.
    filled: bool,
}

impl GlyphAtlas {
    /// A square atlas of `size` px that may grow (doubling) up to `max` px.
    pub fn new(size: u32, max: u32) -> Self {
        GlyphAtlas {
            packer: ShelfPacker::new(size, size),
            size,
            max: max.max(size),
            cache: HashMap::new(),
            generation: 0,
            filled: false,
        }
    }

    /// Look up (rasterizing and uploading on first use) `fonts`' glyph. Misses are
    /// cached too. If the atlas is full it is cleared, or, when it already filled once
    /// this frame (the live glyphs don't fit), doubled up to `max`; either way the
    /// generation bumps and the insert is retried; callers must re-request glyphs after a
    /// generation change. Fonts that are gone leave dead glyphs behind, so one fill a
    /// frame must not grow the atlas.
    pub fn get<T: AtlasTexture>(
        &mut self,
        ch: char,
        style: Style,
        fonts: &mut FontSet,
        tex: &mut T,
    ) -> Option<AtlasGlyph> {
        let key = (fonts.key(), ch, style);
        if let Some(hit) = self.cache.get(&key) {
            return *hit;
        }
        let result = fonts.rasterize(ch, style).and_then(|raster| {
            let (w, h) = (raster.width, raster.height);
            if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
                return None;
            }
            if !self.packer.fits_empty(w, h) {
                return None;
            }
            let (x, y) = match self.packer.insert(w, h) {
                Some(p) => p,
                None => {
                    if self.filled && self.size < self.max {
                        let size = (self.size * 2).min(self.max);
                        if tex.resize(size) {
                            self.size = size;
                            self.packer = ShelfPacker::new(size, size);
                        } else {
                            // Out of memory: restore the old storage and stop growing.
                            tex.resize(self.size);
                            self.max = self.size;
                        }
                    }
                    self.filled = true;
                    self.clear();
                    self.packer.insert(w, h)?
                }
            };
            tex.upload(x, y, w, h, &raster.data);
            Some(AtlasGlyph {
                x: x as u16,
                y: y as u16,
                w: w as u16,
                h: h as u16,
                left: raster.left.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                top: raster.top.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                color: raster.color,
            })
        });
        self.cache.insert(key, result);
        result
    }

    /// Call once at the start of every frame (see `get`'s growth policy).
    pub fn begin_frame(&mut self) {
        self.filled = false;
    }

    pub fn clear(&mut self) {
        self.packer.clear();
        self.cache.clear();
        self.generation += 1;
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FontSpec, system_db};

    #[derive(Default)]
    struct Mock {
        uploads: Vec<(u32, u32, u32, u32, usize)>,
        resizes: Vec<u32>,
        /// Refuse any resize above this side (as out of memory).
        limit: Option<u32>,
    }
    impl AtlasTexture for Mock {
        fn upload(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) {
            self.uploads.push((x, y, w, h, rgba.len()));
        }
        fn resize(&mut self, size: u32) -> bool {
            self.resizes.push(size);
            self.limit.is_none_or(|l| size <= l)
        }
    }

    fn fonts() -> FontSet {
        FontSet::load(&FontSpec::default(), 1.0, system_db()).expect("font load")
    }

    #[test]
    fn packs_left_to_right() {
        let mut p = ShelfPacker::new(16, 16);
        assert_eq!(p.insert(4, 4), Some((0, 0)));
        assert_eq!(p.insert(4, 4), Some((5, 0)));
    }

    #[test]
    fn new_shelf_when_row_full() {
        let mut p = ShelfPacker::new(16, 16);
        assert_eq!(p.insert(7, 4), Some((0, 0)));
        assert_eq!(p.insert(7, 4), Some((8, 0)));
        assert_eq!(p.insert(7, 4), Some((0, 5)));
    }

    #[test]
    fn none_when_full() {
        let mut p = ShelfPacker::new(16, 16);
        assert_eq!(p.insert(17, 4), None);
        assert_eq!(p.insert(16, 16), Some((0, 0)));
        assert_eq!(p.insert(4, 4), None);
        p.clear();
        assert_eq!(p.insert(4, 4), Some((0, 0)));
    }

    #[test]
    fn atlas_caches_glyph() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(256, 256);
        let mut t = Mock::default();
        let g1 = a.get('A', Style::Regular, &mut f, &mut t);
        let g2 = a.get('A', Style::Regular, &mut f, &mut t);
        assert!(g1.is_some());
        assert_eq!(g1, g2);
        assert_eq!(t.uploads.len(), 1);
    }

    #[test]
    fn atlas_caches_misses() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(256, 256);
        let mut t = Mock::default();
        assert_eq!(a.get(' ', Style::Regular, &mut f, &mut t), None);
        assert_eq!(a.get(' ', Style::Regular, &mut f, &mut t), None);
        assert!(t.uploads.is_empty());
    }

    #[test]
    fn atlas_clears_and_retries_when_full() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(32, 32);
        let mut t = Mock::default();
        let g0 = a.generation();
        for ch in ('A'..='Z').chain('a'..='z') {
            assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some(), "{ch}");
        }
        assert!(a.generation() > g0);
        assert!(t.uploads.len() >= 52);
    }

    #[test]
    fn atlas_stops_growing_at_max() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(32, 32);
        let mut t = Mock::default();
        for ch in ('A'..='Z').chain('a'..='z') {
            assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some(), "{ch}");
        }
        assert!(t.resizes.is_empty());
        assert_eq!(a.size(), 32);
    }

    #[test]
    fn atlas_grows_when_full() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(32, 128);
        let mut t = Mock::default();
        for ch in ('A'..='Z').chain('a'..='z') {
            assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some(), "{ch}");
        }
        assert_eq!(t.resizes.first(), Some(&64));
        assert!(a.size() > 32);
        assert_eq!(Some(&a.size()), t.resizes.last());
    }

    #[test]
    fn out_of_memory_keeps_the_old_size_for_good() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(32, 128);
        let mut t = Mock {
            limit: Some(32),
            ..Mock::default()
        };
        for _ in 0..3 {
            for ch in ('A'..='Z').chain('a'..='z') {
                assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some(), "{ch}");
            }
        }
        // One refused growth, then the old storage re-allocated; never tried again.
        assert_eq!(t.resizes, vec![64, 32]);
        assert_eq!(a.size(), 32);
    }

    #[test]
    fn growth_keeps_working_after_full_at_max() {
        let mut f = fonts();
        let mut a = GlyphAtlas::new(32, 64);
        let mut t = Mock::default();
        for _ in 0..3 {
            for ch in ('A'..='Z').chain('a'..='z') {
                assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some(), "{ch}");
            }
        }
        assert_eq!(a.size(), 64);
        assert_eq!(t.resizes, vec![64]);
    }

    fn load_pt(pt: f32) -> FontSet {
        FontSet::load(
            &FontSpec {
                size_pt: pt,
                ..FontSpec::default()
            },
            1.0,
            system_db(),
        )
        .expect("font load")
    }

    #[test]
    fn dead_fonts_do_not_grow_the_atlas() {
        // Zooming loads a set with a fresh key per step; the old sets' glyphs stay behind
        // as garbage. One font per frame fills the atlas at most once a frame, so it
        // clears at its size instead of growing.
        let mut a = GlyphAtlas::new(128, 1024);
        let mut t = Mock::default();
        for i in 0..24 {
            a.begin_frame();
            let mut f = load_pt(11.0 + (i % 2) as f32);
            for ch in 'A'..='Z' {
                assert!(a.get(ch, Style::Regular, &mut f, &mut t).is_some());
            }
        }
        assert!(a.generation() > 0, "the atlas did fill and clear");
        assert!(t.resizes.is_empty());
        assert_eq!(a.size(), 128);
    }

    #[test]
    fn second_fill_in_one_frame_grows() {
        let (mut f1, mut f2) = (load_pt(40.0), load_pt(44.0));
        let mut a = GlyphAtlas::new(256, 4096);
        let mut t = Mock::default();
        // The first frame fills twice and grows; after that nothing is uploaded or cleared.
        let mut settled = None;
        for pass in 0..4 {
            a.begin_frame();
            // As the renderer does: a clear mid-frame makes the frame fetch everything again.
            for _ in 0..3 {
                let generation = a.generation();
                for ch in 'A'..='Z' {
                    assert!(a.get(ch, Style::Regular, &mut f1, &mut t).is_some());
                    assert!(a.get(ch, Style::Regular, &mut f2, &mut t).is_some());
                }
                if a.generation() == generation {
                    break;
                }
            }
            if pass == 1 {
                settled = Some((t.uploads.len(), a.generation()));
            }
        }
        assert_eq!(t.resizes.len(), 1, "grew once: {:?}", t.resizes);
        let (uploads, generation) = settled.unwrap();
        assert_eq!(t.uploads.len(), uploads, "no uploads once settled");
        assert_eq!(a.generation(), generation);
    }

    #[test]
    fn atlas_keys_by_font() {
        let mut small = fonts();
        let mut big = FontSet::load(
            &FontSpec {
                size_pt: 22.0,
                ..FontSpec::default()
            },
            1.0,
            system_db(),
        )
        .expect("font load");
        assert_ne!(small.key(), big.key());
        let mut a = GlyphAtlas::new(256, 256);
        let mut t = Mock::default();
        let gs = a.get('A', Style::Regular, &mut small, &mut t).unwrap();
        let gb = a.get('A', Style::Regular, &mut big, &mut t).unwrap();
        assert_eq!(t.uploads.len(), 2);
        assert_ne!(gs, gb);
        assert!(gb.h > gs.h);
        // Both stay cached.
        assert_eq!(a.get('A', Style::Regular, &mut small, &mut t), Some(gs));
        assert_eq!(a.get('A', Style::Regular, &mut big, &mut t), Some(gb));
        assert_eq!(t.uploads.len(), 2);
    }

    #[test]
    fn oversized_glyph_does_not_clear_atlas() {
        let mut small = fonts();
        let mut big = FontSet::load(&FontSpec::default(), 4.0, system_db()).expect("font load");
        let mut a = GlyphAtlas::new(20, 20);
        let mut t = Mock::default();
        let g = a.get('A', Style::Regular, &mut small, &mut t);
        assert!(g.is_some());
        let gen_before = a.generation();
        let uploads = t.uploads.len();
        assert_eq!(a.get('W', Style::Regular, &mut big, &mut t), None);
        assert_eq!(a.generation(), gen_before);
        assert_eq!(a.get('A', Style::Regular, &mut small, &mut t), g);
        assert_eq!(t.uploads.len(), uploads);
    }
}
