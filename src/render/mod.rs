//! OpenGL renderer: instanced cell and rect pipelines over one glyph atlas, and textured
//! quads for background images.

pub mod atlas;
pub mod cells;
pub mod image;
pub mod rects;

use glow::HasContext;

use crate::font::{FontSet, Metrics, Style};
use crate::layout::Rect;
use crate::term::colors::Rgb;
use crate::term::snapshot::Snapshot;
use atlas::{AtlasTexture, GlyphAtlas};
use cells::{CellInstance, CellPipeline, CellUniforms, CursorStyle};
use image::{ImagePipeline, ImageTexture, Rgba8};
pub use rects::RectInstance;
use rects::RectPipeline;

/// The atlas GL texture, borrowed with its context for uploads.
pub struct GlTexture<'a> {
    pub gl: &'a glow::Context,
    pub tex: glow::Texture,
}

/// Allocate (or reallocate) `tex` as an empty `size` x `size` RGBA texture.
unsafe fn alloc_texture(gl: &glow::Context, tex: glow::Texture, size: u32) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size as i32,
            size as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
    }
}

impl AtlasTexture for GlTexture<'_> {
    fn resize(&mut self, size: u32) -> bool {
        unsafe {
            // Drop stale errors so only this allocation's is read (capped: a lost context
            // can report errors forever).
            for _ in 0..16 {
                if self.gl.get_error() == glow::NO_ERROR {
                    break;
                }
            }
            alloc_texture(self.gl, self.tex, size);
            self.gl.get_error() != glow::OUT_OF_MEMORY
        }
    }

    fn upload(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) {
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            self.gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(rgba)),
            );
        }
    }
}

/// The atlas never grows past this side, whatever the GL allows.
const MAX_ATLAS_SIZE: u32 = 8192;

pub struct Renderer {
    gl: glow::Context,
    rects: RectPipeline,
    cells: CellPipeline,
    images: ImagePipeline,
    atlas: GlyphAtlas,
    atlas_tex: glow::Texture,
    /// GL_MAX_TEXTURE_SIZE (at least 256).
    max_texture: u32,
    width: u32,
    height: u32,
    proj: [f32; 16],
    cell_buf: Vec<CellInstance>,
    rect_buf: Vec<RectInstance>,
    cursor_color: Rgb,
}

impl Renderer {
    pub fn new(gl: glow::Context, atlas_size: u32) -> Result<Renderer, String> {
        unsafe {
            let max = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(256) as u32;
            let atlas_size = atlas_size.min(max);
            let atlas_tex = gl.create_texture()?;
            alloc_texture(&gl, atlas_tex, atlas_size);
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);

            let quad = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad));
            let corners: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&corners), glow::STATIC_DRAW);

            let rects = RectPipeline::new(&gl, quad)?;
            let cells = CellPipeline::new(&gl, quad)?;
            let images = ImagePipeline::new(&gl, quad)?;
            Ok(Renderer {
                gl,
                rects,
                cells,
                images,
                atlas: GlyphAtlas::new(atlas_size, max.min(MAX_ATLAS_SIZE)),
                atlas_tex,
                max_texture: max,
                width: 1,
                height: 1,
                proj: ortho(1, 1),
                cell_buf: Vec::new(),
                rect_buf: Vec::new(),
                cursor_color: Rgb {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            })
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w.max(1);
        self.height = h.max(1);
        self.proj = ortho(self.width, self.height);
    }

    /// Color of beam, underline and hollow cursors.
    pub fn set_cursor_color(&mut self, c: Rgb) {
        self.cursor_color = c;
    }

    /// Clear to `clear` (RGBA; `(0,0,0,0)` when any visible pane is translucent).
    pub fn begin_frame(&mut self, clear: [f32; 4]) {
        self.atlas.begin_frame();
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, self.width as i32, self.height as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            // Sources are straight (non-premultiplied) alpha. The framebuffer holds
            // premultiplied alpha, which is what X11 and Wayland compositors expect:
            //   rgb = src.rgb·src.a + dst.rgb·(1 − src.a),  a = src.a + dst.a·(1 − src.a)
            // is exactly premultiplied "over" with the source premultiplied (src.rgb·src.a,
            // src.a). A transparent (0,0,0,0) or opaque clear is valid premultiplied, and
            // "over" keeps every pixel valid (rgb ≤ a), so translucent panes, the dim, the
            // overlay and glyph coverage all composite correctly. Over an opaque dst, a stays 1.
            gl.blend_func_separate(
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
            );
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Draw a pane's cells at `content`; nothing is drawn outside `clip` (the pane's
    /// rect), so a cell larger than a tiny pane cannot paint over its neighbours.
    /// Default-bg cells get bg alpha `default_bg_alpha` (see `cells::build_instances`).
    // Per-pane inputs; bundling them would only rename the argument list.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_pane(
        &mut self,
        clip: Rect,
        content: Rect,
        snap: &Snapshot,
        fonts: &mut FontSet,
        m: &Metrics,
        focused: bool,
        default_bg_alpha: u8,
    ) {
        // If the atlas clears or grows mid-build, glyphs fetched earlier are stale: rebuild.
        // Panes with other fonts share the atlas, and a later pane's build may clear it and
        // upload over this pane's glyphs. That is fine: this pane's draw call is issued
        // below with the UVs valid now, and GL runs a texture upload after the draws issued
        // before it, so only glyphs fetched within one build can go stale. Growth
        // reallocates the texture (tex_image_2d) but is ordered the same way: draws issued
        // earlier carry their own atlas_size uniform and sample the old storage, and the
        // size below is read after the rebuild loop, so this pane's draw sees the new one.
        // Up to three builds: a first fill clears, a second fill in the same frame grows.
        for _ in 0..3 {
            let generation = self.atlas.generation();
            let atlas = &mut self.atlas;
            let mut tex = GlTexture {
                gl: &self.gl,
                tex: self.atlas_tex,
            };
            cells::build_instances(
                snap,
                content,
                m,
                CursorStyle {
                    focused,
                    color: self.cursor_color,
                },
                |ch, style| atlas.get(ch, style, fonts, &mut tex),
                &mut self.cell_buf,
                &mut self.rect_buf,
                default_bg_alpha,
            );
            if self.atlas.generation() == generation {
                break;
            }
        }
        let uniforms = CellUniforms {
            proj: &self.proj,
            origin: (content.x, content.y),
            metrics: m,
            atlas_size: self.atlas.size(),
            atlas_tex: self.atlas_tex,
        };
        let (x, y, w, h) = scissor_box(clip, self.height);
        unsafe {
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.scissor(x, y, w, h);
        }
        self.cells.draw(&self.gl, &uniforms, &self.cell_buf);
        self.rects.draw(&self.gl, &self.proj, &self.rect_buf);
        unsafe { self.gl.disable(glow::SCISSOR_TEST) };
    }

    /// Draw one row of text with its top-left cell at (`x`, `y`), over a transparent
    /// background, scissored to `clip`.
    // The 8-argument signature is the plan's interface.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        fg: Rgb,
        clip: Rect,
        fonts: &mut FontSet,
        m: &Metrics,
    ) {
        let laid_out = crate::ui::text::layout_cells(text);
        // If the atlas clears or grows mid-build, glyphs fetched earlier are stale: rebuild.
        // Up to three builds: a first fill clears, a second fill in the same frame grows.
        for _ in 0..3 {
            let generation = self.atlas.generation();
            let atlas = &mut self.atlas;
            let mut tex = GlTexture {
                gl: &self.gl,
                tex: self.atlas_tex,
            };
            cells::text_instances(
                &laid_out,
                fg,
                |ch| atlas.get(ch, Style::Regular, fonts, &mut tex),
                &mut self.cell_buf,
            );
            if self.atlas.generation() == generation {
                break;
            }
        }
        let uniforms = CellUniforms {
            proj: &self.proj,
            origin: (x, y),
            metrics: m,
            atlas_size: self.atlas.size(),
            atlas_tex: self.atlas_tex,
        };
        let (sx, sy, sw, sh) = scissor_box(clip, self.height);
        unsafe {
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.scissor(sx, sy, sw, sh);
        }
        self.cells.draw(&self.gl, &uniforms, &self.cell_buf);
        unsafe { self.gl.disable(glow::SCISSOR_TEST) };
    }

    pub fn draw_rects(&mut self, rects: &[RectInstance]) {
        self.rects.draw(&self.gl, &self.proj, rects);
    }

    /// The largest texture side the GL supports; images are downscaled to fit it.
    pub fn max_texture_size(&self) -> u32 {
        self.max_texture
    }

    /// Upload a background image as a texture (shared by the panes that show it).
    pub fn upload_image(&mut self, img: &Rgba8) -> Result<ImageTexture, String> {
        ImagePipeline::upload(&self.gl, img)
    }

    pub fn delete_image(&mut self, t: ImageTexture) {
        ImagePipeline::delete(&self.gl, t);
    }

    /// Draw `t`'s `uv` range (see `image::fit_rect`) into `dest` at `alpha`; `repeat`
    /// tiles a uv beyond 0..1.
    pub fn draw_image(
        &mut self,
        t: &ImageTexture,
        dest: Rect,
        uv: [f32; 4],
        alpha: f32,
        repeat: bool,
    ) {
        let proj = self.proj;
        self.images
            .draw(&self.gl, &proj, t, dest, uv, alpha, repeat);
    }

    /// Drop every cached glyph (font or scale changed).
    pub fn clear_glyphs(&mut self) {
        self.atlas.clear();
    }
}

/// `glScissor` box (x, y from the bottom, w, h) for a top-left-origin rect in a target `height` px tall.
fn scissor_box(r: Rect, height: u32) -> (i32, i32, i32, i32) {
    let (w, h) = (r.w.max(0), r.h.max(0));
    let height = height.min(i32::MAX as u32) as i32;
    (r.x, height - (r.y + h), w, h)
}

/// Column-major orthographic projection: pixels (origin top-left) to clip space.
fn ortho(w: u32, h: u32) -> [f32; 16] {
    let (sx, sy) = (2.0 / w as f32, -2.0 / h as f32);
    [
        sx, 0.0, 0.0, 0.0, //
        0.0, sy, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0,
    ]
}

/// View plain-old-data GPU vertex structs (repr(C), no padding bytes) as bytes.
fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), size_of_val(v)) }
}

/// Bind the shared unit quad to attribute 0 of the currently bound VAO.
fn bind_quad(gl: &glow::Context, quad: glow::Buffer) {
    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
    }
}

/// Prefix `src` with the GLSL version line (and GLES precision) for `stage`.
fn shader_source(stage: u32, embedded: bool, src: &str) -> String {
    // GLES vertex shaders default to highp; mediump there would quantize pixel coords.
    let header = match (embedded, stage == glow::FRAGMENT_SHADER) {
        (true, true) => "#version 300 es\nprecision mediump float;\n",
        (true, false) => "#version 300 es\n",
        (false, _) => "#version 330 core\n",
    };
    format!("{header}{src}")
}

/// Compile and link a program, prefixing the GLSL version for desktop GL or GLES.
fn program(gl: &glow::Context, vert: &str, frag: &str) -> Result<glow::Program, String> {
    let embedded = gl.version().is_embedded;
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vert), (glow::FRAGMENT_SHADER, frag)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &shader_source(kind, embedded, src));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!(
                    "shader compile: {}",
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for s in shaders {
            gl.detach_shader(program, s);
            gl.delete_shader(s);
        }
        if !gl.get_program_link_status(program) {
            return Err(format!("shader link: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_box_flips_y_and_clamps() {
        let r = Rect {
            x: 10,
            y: 20,
            w: 100,
            h: 50,
        };
        // GL's origin is bottom-left: y = 600 - (20 + 50).
        assert_eq!(scissor_box(r, 600), (10, 530, 100, 50));
        let z = Rect { w: -3, h: -1, ..r };
        assert_eq!(scissor_box(z, 600), (10, 580, 0, 0));
    }

    #[test]
    fn desktop_shaders_use_glsl_330_core() {
        for stage in [glow::VERTEX_SHADER, glow::FRAGMENT_SHADER] {
            let s = shader_source(stage, false, "void main() {}");
            assert!(s.starts_with("#version 330 core\n"), "{s}");
            assert!(s.ends_with("void main() {}"));
        }
    }

    #[test]
    fn gles_mediump_only_in_fragment_stage() {
        // The vertex stage defaults to highp; mediump would quantize pixel coords above 1024.
        let vert = shader_source(glow::VERTEX_SHADER, true, "void main() {}");
        assert!(vert.starts_with("#version 300 es\n"), "{vert}");
        assert!(!vert.contains("mediump"), "{vert}");
        let frag = shader_source(glow::FRAGMENT_SHADER, true, "void main() {}");
        assert!(
            frag.starts_with("#version 300 es\nprecision mediump float;\n"),
            "{frag}"
        );
    }
}
