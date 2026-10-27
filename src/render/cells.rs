//! Per-cell instances for the two-pass (background, glyph) cell pipeline.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::CursorShape;

use glow::HasContext;

use crate::font::{Metrics, Style};
use crate::layout::Rect;
use crate::render::RectInstance;
use crate::render::atlas::AtlasGlyph;
use crate::term::colors::Rgb;
use crate::term::snapshot::{SnapCell, Snapshot};

/// Glyph is RGBA color (emoji): drawn untinted.
pub const COLOR_GLYPH: u8 = 1;
/// Cell background spans two cells.
pub const WIDE: u8 = 2;

/// One GPU instance; layout matches the vertex attributes in `cell.vert`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellInstance {
    pub col: u16,
    pub row: u16,
    pub bg: [u8; 4],
    pub fg: [u8; 4],
    /// Atlas x, y, w, h in texels; w == 0 means no glyph.
    pub uv: [u16; 4],
    /// Glyph bitmap placement: left, top (relative to pen position / baseline).
    pub offset: [i16; 2],
    pub flags: u8,
    pub _pad: [u8; 3],
}

/// How a pane's cursor is drawn.
#[derive(Clone, Copy, Debug)]
pub struct CursorStyle {
    /// Unfocused panes get a hollow block.
    pub focused: bool,
    /// Color of beam, underline and hollow cursors.
    pub color: Rgb,
}

/// Fill `out` with one instance per visible cell (wide spacers skipped, one extra
/// instance per zero-width char) and `rects` with decorations and non-block cursors
/// (drawn in `cursor_style.color`; a focused block cursor swaps the cell's fg/bg instead).
/// Cells on the default background get bg alpha `default_bg_alpha` (0 when the pane's
/// translucent bg rect already painted it); every other bg, and the block cursor's
/// swapped cell, stays opaque.
// The 8-argument signature is the plan's interface.
#[allow(clippy::too_many_arguments)]
pub fn build_instances(
    snap: &Snapshot,
    content: Rect,
    m: &Metrics,
    cursor_style: CursorStyle,
    glyph: impl FnMut(char, Style) -> Option<AtlasGlyph>,
    out: &mut Vec<CellInstance>,
    rects: &mut Vec<RectInstance>,
    default_bg_alpha: u8,
) {
    out.clear();
    rects.clear();
    let cols = snap.cols;
    if cols == 0 {
        return;
    }
    let mut glyph = glyph;
    let cursor = snap
        .cursor
        .filter(|c| c.shape != CursorShape::Hidden)
        .map(|c| {
            let mut idx = c.line * cols + c.col;
            // The cursor may sit on a wide char's spacer: draw it on the wide char.
            if c.col > 0 && snap.cells[idx].flags.contains(Flags::WIDE_CHAR_SPACER) {
                idx -= 1;
            }
            (idx, c.shape)
        });
    let block = match cursor {
        Some((idx, CursorShape::Block)) if cursor_style.focused => Some(idx),
        _ => None,
    };
    let geo = Geo {
        content,
        cw: m.cell_width as i32,
        ch: m.cell_height as i32,
        baseline: m.baseline as i32,
    };

    for (idx, cell) in snap.cells.iter().enumerate() {
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let block_cursor = block == Some(idx);
        let (fg, mut bg) = colors(cell, block_cursor);
        if cell.default_bg && !block_cursor {
            bg[3] = default_bg_alpha;
        }
        let g = if cell.ch == ' ' {
            None
        } else {
            glyph(cell.ch, Style::from_flags(cell.flags))
        };
        out.push(instance(idx, cols, cell, fg, bg, g));
        geo.decorations(idx, cols, cell, fg, rects);
    }

    for (idx, chars) in &snap.zerowidth {
        let cell = &snap.cells[*idx];
        let (fg, _) = colors(cell, block == Some(*idx));
        for &zw in chars {
            let g = glyph(zw, Style::from_flags(cell.flags));
            out.push(instance(*idx, cols, cell, fg, [0; 4], g));
        }
    }

    if let Some((idx, shape)) = cursor {
        geo.cursor(idx, cols, &snap.cells[idx], shape, cursor_style, rects);
    }
}

/// One instance per laid-out char of a single text row (row 0), with a transparent
/// background. `glyph` looks up the atlas entry for a char.
pub fn text_instances(
    cells: &[(u16, char, bool)],
    fg: Rgb,
    mut glyph: impl FnMut(char) -> Option<AtlasGlyph>,
    out: &mut Vec<CellInstance>,
) {
    out.clear();
    for &(col, ch, wide) in cells {
        let g = if ch == ' ' { None } else { glyph(ch) };
        let mut flags = if wide { WIDE } else { 0 };
        let (uv, offset) = match g {
            Some(g) => {
                if g.color {
                    flags |= COLOR_GLYPH;
                }
                ([g.x, g.y, g.w, g.h], [g.left, g.top])
            }
            None => ([0; 4], [0; 2]),
        };
        out.push(CellInstance {
            col,
            row: 0,
            bg: [0; 4],
            fg: rgba(fg),
            uv,
            offset,
            flags,
            _pad: [0; 3],
        });
    }
}

fn rgba(c: Rgb) -> [u8; 4] {
    [c.r, c.g, c.b, 255]
}

fn rgba_f32(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| f32::from(v) / 255.0)
}

fn colors(cell: &SnapCell, block_cursor: bool) -> ([u8; 4], [u8; 4]) {
    let (fg, bg) = (rgba(cell.fg), rgba(cell.bg));
    if block_cursor { (bg, fg) } else { (fg, bg) }
}

fn cell_span(cell: &SnapCell) -> i32 {
    if cell.flags.contains(Flags::WIDE_CHAR) {
        2
    } else {
        1
    }
}

fn instance(
    idx: usize,
    cols: usize,
    cell: &SnapCell,
    fg: [u8; 4],
    bg: [u8; 4],
    g: Option<AtlasGlyph>,
) -> CellInstance {
    let mut flags = 0;
    if cell.flags.contains(Flags::WIDE_CHAR) {
        flags |= WIDE;
    }
    let (uv, offset) = match g {
        Some(g) => {
            if g.color {
                flags |= COLOR_GLYPH;
            }
            ([g.x, g.y, g.w, g.h], [g.left, g.top])
        }
        None => ([0; 4], [0; 2]),
    };
    CellInstance {
        col: (idx % cols) as u16,
        row: (idx / cols) as u16,
        bg,
        fg,
        uv,
        offset,
        flags,
        _pad: [0; 3],
    }
}

/// Pixel geometry of one pane's cells.
struct Geo {
    content: Rect,
    cw: i32,
    ch: i32,
    baseline: i32,
}

impl Geo {
    /// Pixel rect of the cell (two cells wide for wide chars).
    fn cell_rect(&self, idx: usize, cols: usize, cell: &SnapCell) -> Rect {
        Rect {
            x: self.content.x + (idx % cols) as i32 * self.cw,
            y: self.content.y + (idx / cols) as i32 * self.ch,
            w: self.cw * cell_span(cell),
            h: self.ch,
        }
    }

    fn line_thickness(&self) -> i32 {
        (self.ch / 16).max(1)
    }

    fn decorations(
        &self,
        idx: usize,
        cols: usize,
        cell: &SnapCell,
        fg: [u8; 4],
        rects: &mut Vec<RectInstance>,
    ) {
        if !(cell.link
            || cell
                .flags
                .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT))
        {
            return;
        }
        let r = self.cell_rect(idx, cols, cell);
        let t = self.line_thickness();
        let color = rgba_f32(fg);
        let mut line = |dy: i32| {
            rects.push(RectInstance {
                rect: Rect {
                    x: r.x,
                    y: r.y + dy.clamp(0, (self.ch - t).max(0)),
                    w: r.w,
                    h: t,
                },
                color,
            })
        };
        if cell.link || cell.flags.intersects(Flags::ALL_UNDERLINES) {
            let below = (self.ch - self.baseline) / 3;
            let dy = self.baseline + below.max(t);
            line(dy);
            if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
                let second = dy + 2 * t;
                line(if second + t <= self.ch {
                    second
                } else {
                    dy - 2 * t
                });
            }
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            line(self.baseline - self.baseline * 3 / 10);
        }
    }

    fn cursor(
        &self,
        idx: usize,
        cols: usize,
        cell: &SnapCell,
        shape: CursorShape,
        style: CursorStyle,
        rects: &mut Vec<RectInstance>,
    ) {
        let r = self.cell_rect(idx, cols, cell);
        let color = rgba_f32(rgba(style.color));
        let thin = ((self.cw as f32 * 0.15).round() as i32).max(1);
        let mut push = |x, y, w, h| {
            rects.push(RectInstance {
                rect: Rect { x, y, w, h },
                color,
            })
        };
        if !style.focused || shape == CursorShape::HollowBlock {
            push(r.x, r.y, r.w, 1);
            push(r.x, r.y + r.h - 1, r.w, 1);
            push(r.x, r.y + 1, 1, r.h - 2);
            push(r.x + r.w - 1, r.y + 1, 1, r.h - 2);
            return;
        }
        match shape {
            CursorShape::Beam => push(r.x, r.y, thin, r.h),
            CursorShape::Underline => push(r.x, r.y + r.h - thin, r.w, thin),
            // Block: drawn by swapping the cell's colors.
            _ => {}
        }
    }
}

pub(crate) struct CellPipeline {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_proj: Option<glow::UniformLocation>,
    u_origin: Option<glow::UniformLocation>,
    u_cell: Option<glow::UniformLocation>,
    u_baseline: Option<glow::UniformLocation>,
    u_atlas: Option<glow::UniformLocation>,
    u_pass: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
}

/// Per-draw uniforms for one pane.
pub(crate) struct CellUniforms<'a> {
    pub proj: &'a [f32; 16],
    pub origin: (i32, i32),
    pub metrics: &'a Metrics,
    pub atlas_size: u32,
    pub atlas_tex: glow::Texture,
}

impl CellPipeline {
    pub(crate) fn new(gl: &glow::Context, quad: glow::Buffer) -> Result<Self, String> {
        let program = super::program(
            gl,
            include_str!("shaders/cell.vert"),
            include_str!("shaders/cell.frag"),
        )?;
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            super::bind_quad(gl, quad);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = size_of::<CellInstance>() as i32;
            // (location, components, type, normalized, byte offset)
            let float_attrs = [
                (1u32, 2, glow::UNSIGNED_SHORT, false, 0i32),
                (2, 4, glow::UNSIGNED_BYTE, true, 4),
                (3, 4, glow::UNSIGNED_BYTE, true, 8),
                (4, 4, glow::UNSIGNED_SHORT, false, 12),
                (5, 2, glow::SHORT, false, 20),
            ];
            for (loc, n, ty, norm, offset) in float_attrs {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, n, ty, norm, stride, offset);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.enable_vertex_attrib_array(6);
            gl.vertex_attrib_pointer_i32(6, 1, glow::UNSIGNED_BYTE, stride, 24);
            gl.vertex_attrib_divisor(6, 1);
            gl.bind_vertex_array(None);
            let loc = |name| gl.get_uniform_location(program, name);
            Ok(CellPipeline {
                u_proj: loc("u_proj"),
                u_origin: loc("u_origin"),
                u_cell: loc("u_cell"),
                u_baseline: loc("u_baseline"),
                u_atlas: loc("u_atlas"),
                u_pass: loc("u_pass"),
                u_tex: loc("u_tex"),
                program,
                vao,
                vbo,
            })
        }
    }

    /// Upload `instances` and draw backgrounds, then glyphs.
    pub(crate) fn draw(&self, gl: &glow::Context, u: &CellUniforms, instances: &[CellInstance]) {
        if instances.is_empty() {
            return;
        }
        let m = u.metrics;
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_proj.as_ref(), false, u.proj);
            gl.uniform_2_f32(self.u_origin.as_ref(), u.origin.0 as f32, u.origin.1 as f32);
            let (cw, ch) = (m.cell_width as f32, m.cell_height as f32);
            gl.uniform_2_f32(self.u_cell.as_ref(), cw, ch);
            gl.uniform_1_f32(self.u_baseline.as_ref(), m.baseline as f32);
            let atlas = u.atlas_size as f32;
            gl.uniform_2_f32(self.u_atlas.as_ref(), atlas, atlas);
            gl.uniform_1_i32(self.u_tex.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(u.atlas_tex));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                super::as_bytes(instances),
                glow::STREAM_DRAW,
            );
            for pass in 0..2 {
                gl.uniform_1_i32(self.u_pass.as_ref(), pass);
                gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, instances.len() as i32);
            }
            gl.bind_vertex_array(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::colors::{Rgb, tango_default};
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    const M: Metrics = Metrics {
        cell_width: 8,
        cell_height: 16,
        baseline: 12,
    };
    const CONTENT: Rect = Rect {
        x: 4,
        y: 4,
        w: 80,
        h: 48,
    };

    const CURSOR: Rgb = Rgb { r: 1, g: 2, b: 3 };
    const CURSOR_F32: [f32; 4] = [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 1.0];

    fn snap(bytes: &[u8]) -> Snapshot {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, bytes);
        let mut s = Snapshot::default();
        s.capture(
            &term,
            &tango_default(),
            false,
            &crate::term::snapshot::Highlights::NONE,
        );
        s
    }

    fn fake_glyph(ch: char, _: Style) -> Option<AtlasGlyph> {
        (ch != ' ').then_some(AtlasGlyph {
            x: 10,
            y: 20,
            w: 6,
            h: 9,
            left: 1,
            top: 9,
            color: ch == '😀',
        })
    }

    fn build(s: &Snapshot, focused: bool) -> (Vec<CellInstance>, Vec<RectInstance>) {
        let (mut out, mut rects) = (Vec::new(), Vec::new());
        let style = CursorStyle {
            focused,
            color: CURSOR,
        };
        build_instances(s, CONTENT, &M, style, fake_glyph, &mut out, &mut rects, 255);
        (out, rects)
    }

    fn rgba(c: Rgb) -> [u8; 4] {
        [c.r, c.g, c.b, 255]
    }

    fn at(out: &[CellInstance], col: u16, row: u16) -> Vec<CellInstance> {
        out.iter()
            .copied()
            .filter(|i| i.col == col && i.row == row)
            .collect()
    }

    #[test]
    fn instance_layout_matches_vertex_attributes() {
        // cell.vert reads col/row @0, bg @4, fg @8, uv @12, offset @20, flags @24.
        assert_eq!(size_of::<CellInstance>(), 28);
        assert_eq!(std::mem::offset_of!(CellInstance, uv), 12);
        assert_eq!(std::mem::offset_of!(CellInstance, offset), 20);
        assert_eq!(std::mem::offset_of!(CellInstance, flags), 24);
    }

    #[test]
    fn one_instance_per_cell_with_glyph_placement() {
        let s = snap(b"\x1b[?25la");
        let (out, rects) = build(&s, true);
        assert_eq!(out.len(), 30);
        assert!(rects.is_empty());
        let a = at(&out, 0, 0)[0];
        assert_eq!(a.uv, [10, 20, 6, 9]);
        assert_eq!(a.offset, [1, 9]);
        assert_eq!(a.flags, 0);
        let p = tango_default();
        assert_eq!((a.fg, a.bg), (rgba(p.fg), rgba(p.bg)));
        assert_eq!(at(&out, 1, 0)[0].uv[2], 0, "space has no glyph");
    }

    #[test]
    fn wide_char_spans_two_cells_and_spacer_is_skipped() {
        let s = snap("\x1b[?25l中😀".as_bytes());
        let (out, _) = build(&s, true);
        assert_eq!(at(&out, 0, 0)[0].flags, WIDE);
        assert!(at(&out, 1, 0).is_empty());
        assert_eq!(at(&out, 2, 0)[0].flags, WIDE | COLOR_GLYPH);
        assert!(at(&out, 3, 0).is_empty());
        assert_eq!(out.len(), 28);
    }

    #[test]
    fn zerowidth_is_extra_glyph_without_background() {
        let s = snap("\x1b[?25le\u{301}".as_bytes());
        let (out, _) = build(&s, true);
        let cell = at(&out, 0, 0);
        assert_eq!(cell.len(), 2);
        assert_eq!(cell[1].bg[3], 0);
        assert_eq!(cell[1].fg, cell[0].fg);
        assert_eq!(cell[1].uv, [10, 20, 6, 9]);
    }

    #[test]
    fn block_cursor_swaps_cell_colors() {
        let s = snap(b"ab");
        let (out, rects) = build(&s, true);
        let p = tango_default();
        let c = at(&out, 2, 0)[0];
        assert_eq!((c.fg, c.bg), (rgba(p.bg), rgba(p.fg)));
        assert!(rects.is_empty());
    }

    #[test]
    fn default_bg_alpha_applied() {
        // Cursor on the default-bg cell at col 2: its swapped (block) bg stays opaque.
        let s = snap(b"a\x1b[41mb\x1b[0m");
        let (mut out, mut rects) = (Vec::new(), Vec::new());
        let style = CursorStyle {
            focused: true,
            color: CURSOR,
        };
        build_instances(&s, CONTENT, &M, style, fake_glyph, &mut out, &mut rects, 0);
        let p = tango_default();
        assert_eq!(at(&out, 0, 0)[0].bg, [p.bg.r, p.bg.g, p.bg.b, 0]);
        assert_eq!(at(&out, 1, 0)[0].bg, rgba(p.ansi[1]));
        assert_eq!(
            at(&out, 2, 0)[0].bg,
            rgba(p.fg),
            "block cursor stays opaque"
        );
        assert_eq!(at(&out, 3, 0)[0].bg[3], 0);
        assert_eq!(at(&out, 0, 2)[0].bg[3], 0);
        let opaque = out.iter().filter(|i| i.bg[3] == 255).count();
        assert_eq!((out.len(), opaque), (30, 2));
    }

    #[test]
    fn beam_cursor_is_a_thin_rect() {
        let s = snap(b"ab\x1b[6 q");
        let (out, rects) = build(&s, true);
        let p = tango_default();
        assert_eq!(at(&out, 2, 0)[0].bg, rgba(p.bg));
        assert_eq!(rects.len(), 1);
        let r = rects[0].rect;
        assert_eq!((r.x, r.y, r.h), (4 + 16, 4, 16));
        assert!(r.w >= 1 && r.w < 8);
        assert_eq!(rects[0].color, CURSOR_F32);
    }

    #[test]
    fn underline_cursor_is_a_rect_at_cell_bottom() {
        let s = snap(b"\x1b[4 q");
        let (_, rects) = build(&s, true);
        assert_eq!(rects.len(), 1);
        let r = rects[0].rect;
        assert_eq!((r.x, r.w), (4, 8));
        assert_eq!(r.y + r.h, 4 + 16);
        assert_eq!(rects[0].color, CURSOR_F32);
    }

    #[test]
    fn unfocused_cursor_is_hollow_block() {
        let s = snap(b"ab");
        let (out, rects) = build(&s, false);
        let p = tango_default();
        assert_eq!(at(&out, 2, 0)[0].bg, rgba(p.bg));
        assert_eq!(rects.len(), 4);
        let x0 = rects.iter().map(|r| r.rect.x).min().unwrap();
        let x1 = rects.iter().map(|r| r.rect.x + r.rect.w).max().unwrap();
        let y1 = rects.iter().map(|r| r.rect.y + r.rect.h).max().unwrap();
        assert_eq!((x0, x1, y1), (4 + 16, 4 + 24, 4 + 16));
        assert!(rects.iter().all(|r| r.color == CURSOR_F32));
    }

    #[test]
    fn underline_and_strikeout_become_rects() {
        let s = snap(b"\x1b[?25l\x1b[4mU\x1b[0;9mS");
        let (_, rects) = build(&s, true);
        assert_eq!(rects.len(), 2);
        let p = tango_default();
        let fg = [
            f32::from(p.fg.r) / 255.0,
            f32::from(p.fg.g) / 255.0,
            f32::from(p.fg.b) / 255.0,
            1.0,
        ];
        let (u, st) = (rects[0], rects[1]);
        assert_eq!((u.rect.x, u.rect.w, u.color), (4, 8, fg));
        assert!(u.rect.y > 4 + 12 - 1 && u.rect.y + u.rect.h <= 4 + 16);
        assert_eq!((st.rect.x, st.rect.w), (12, 8));
        assert!(st.rect.y > 4 && st.rect.y < 4 + 12);
    }

    #[test]
    fn text_instances_wide_color_and_transparent_bg() {
        let cells = [(0, 'a', false), (1, '😀', true), (3, ' ', false)];
        let mut out = Vec::new();
        text_instances(&cells, CURSOR, |c| fake_glyph(c, Style::Regular), &mut out);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|i| i.row == 0 && i.bg[3] == 0));
        assert_eq!(out[0].fg, [1, 2, 3, 255]);
        assert_eq!((out[0].flags, out[0].uv), (0, [10, 20, 6, 9]));
        assert_eq!((out[1].col, out[1].flags), (1, WIDE | COLOR_GLYPH));
        assert_eq!(out[2].uv[2], 0);
    }
}
