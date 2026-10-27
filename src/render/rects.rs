//! Instanced solid rectangles (cursor shapes, decorations, chrome).

use crate::layout::Rect;

/// A pixel rect with a straight-alpha RGBA color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectInstance {
    pub rect: Rect,
    pub color: [f32; 4],
}

use glow::HasContext;

/// Floats per GPU rect instance: x, y, w, h, r, g, b, a.
const FLOATS: usize = 8;

pub(crate) struct RectPipeline {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_proj: Option<glow::UniformLocation>,
    scratch: Vec<f32>,
}

impl RectPipeline {
    pub(crate) fn new(gl: &glow::Context, quad: glow::Buffer) -> Result<Self, String> {
        let program = super::program(
            gl,
            include_str!("shaders/rect.vert"),
            include_str!("shaders/rect.frag"),
        )?;
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            super::bind_quad(gl, quad);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (FLOATS * 4) as i32;
            for (loc, offset) in [(1u32, 0i32), (2, 16)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 4, glow::FLOAT, false, stride, offset);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);
            Ok(RectPipeline {
                u_proj: gl.get_uniform_location(program, "u_proj"),
                program,
                vao,
                vbo,
                scratch: Vec::new(),
            })
        }
    }

    pub(crate) fn draw(&mut self, gl: &glow::Context, proj: &[f32; 16], rects: &[RectInstance]) {
        if rects.is_empty() {
            return;
        }
        self.scratch.clear();
        for r in rects {
            let Rect { x, y, w, h } = r.rect;
            self.scratch
                .extend_from_slice(&[x as f32, y as f32, w.max(0) as f32, h.max(0) as f32]);
            self.scratch.extend_from_slice(&r.color);
        }
        unsafe {
            let bytes = super::as_bytes(&self.scratch);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_proj.as_ref(), false, proj);
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, rects.len() as i32);
            gl.bind_vertex_array(None);
        }
    }
}
