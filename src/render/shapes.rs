//! Instanced antialiased strokes (chrome icons): round-capped lines, rounded-rect
//! outlines and rings, each shaded from its distance field in the fragment shader.

use crate::layout::Rect;

/// Stroke geometry in pixels (centre lines; the stroke is centred on them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A line from `a` to `b` with round caps; `a == b` is a dot.
    Line { a: [f32; 2], b: [f32; 2] },
    /// The outline of a rectangle with corner radius `r`.
    RoundRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
    },
    /// A circle of radius `r` around `c`.
    Ring { c: [f32; 2], r: f32 },
}

/// A shape stroked `width` px wide in a straight-alpha RGBA color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeInstance {
    pub shape: Shape,
    pub width: f32,
    pub color: [f32; 4],
}

impl ShapeInstance {
    /// The whole pixels the stroke can touch, with one px for the antialiased edge.
    pub fn bounds(&self) -> Rect {
        let (x0, y0, x1, y1) = match self.shape {
            Shape::Line { a, b } => (
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ),
            Shape::RoundRect { x, y, w, h, .. } => (x, y, x + w, y + h),
            Shape::Ring { c, r } => (c[0] - r, c[1] - r, c[0] + r, c[1] + r),
        };
        let m = self.width / 2.0 + 1.0;
        let (x0, y0) = ((x0 - m).floor() as i32, (y0 - m).floor() as i32);
        let (x1, y1) = ((x1 + m).ceil() as i32, (y1 + m).ceil() as i32);
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    /// GPU floats: the quad, then geometry relative to the quad's corner (small numbers, so
    /// the fragment stage's mediump on GLES keeps sub-pixel precision anywhere on a 4K
    /// window), then kind, radius, half width, and the color.
    fn encode(&self, out: &mut Vec<f32>) {
        let q = self.bounds();
        let (ox, oy) = (q.x as f32, q.y as f32);
        let (geom, kind, r) = match self.shape {
            Shape::Line { a, b } => ([a[0] - ox, a[1] - oy, b[0] - ox, b[1] - oy], 0.0, 0.0),
            Shape::RoundRect { x, y, w, h, r } => {
                ([x - ox, y - oy, w, h], 1.0, r.min(w / 2.0).min(h / 2.0))
            }
            Shape::Ring { c, r } => ([c[0] - ox, c[1] - oy, 0.0, 0.0], 2.0, r),
        };
        out.extend_from_slice(&[ox, oy, q.w as f32, q.h as f32]);
        out.extend_from_slice(&geom);
        out.extend_from_slice(&[kind, r, self.width / 2.0, 0.0]);
        out.extend_from_slice(&self.color);
    }
}

use glow::HasContext;

/// Floats per GPU instance: quad, geometry, params, color.
const FLOATS: usize = 16;

pub(crate) struct ShapePipeline {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_proj: Option<glow::UniformLocation>,
    scratch: Vec<f32>,
}

impl ShapePipeline {
    pub(crate) fn new(gl: &glow::Context, quad: glow::Buffer) -> Result<Self, String> {
        let program = super::program(
            gl,
            include_str!("shaders/shape.vert"),
            include_str!("shaders/shape.frag"),
        )?;
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            super::bind_quad(gl, quad);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (FLOATS * 4) as i32;
            for (loc, offset) in [(1u32, 0i32), (2, 16), (3, 32), (4, 48)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 4, glow::FLOAT, false, stride, offset);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);
            Ok(ShapePipeline {
                u_proj: gl.get_uniform_location(program, "u_proj"),
                program,
                vao,
                vbo,
                scratch: Vec::new(),
            })
        }
    }

    pub(crate) fn draw(&mut self, gl: &glow::Context, proj: &[f32; 16], shapes: &[ShapeInstance]) {
        if shapes.is_empty() {
            return;
        }
        self.scratch.clear();
        for s in shapes {
            s.encode(&mut self.scratch);
        }
        unsafe {
            let bytes = super::as_bytes(&self.scratch);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_proj.as_ref(), false, proj);
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, shapes.len() as i32);
            gl.bind_vertex_array(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(shape: Shape, width: f32) -> ShapeInstance {
        ShapeInstance {
            shape,
            width,
            color: [1.0; 4],
        }
    }

    #[test]
    fn bounds_cover_stroke_and_edge() {
        let l = inst(
            Shape::Line {
                a: [10.0, 20.0],
                b: [4.0, 20.0],
            },
            2.0,
        );
        // x 4..10 and y 20, widened by half the width plus one px of antialiasing.
        assert_eq!(
            l.bounds(),
            Rect {
                x: 2,
                y: 18,
                w: 10,
                h: 4
            }
        );
        let r = inst(
            Shape::Ring {
                c: [8.5, 8.5],
                r: 3.0,
            },
            1.0,
        );
        assert_eq!(
            r.bounds(),
            Rect {
                x: 4,
                y: 4,
                w: 9,
                h: 9
            }
        );
        let b = inst(
            Shape::RoundRect {
                x: 1.0,
                y: 1.0,
                w: 14.0,
                h: 14.0,
                r: 2.0,
            },
            2.0,
        );
        assert_eq!(
            b.bounds(),
            Rect {
                x: -1,
                y: -1,
                w: 18,
                h: 18
            }
        );
    }

    #[test]
    fn encode_is_relative_to_the_quad() {
        let mut v = Vec::new();
        let s = ShapeInstance {
            shape: Shape::Line {
                a: [1000.0, 2000.0],
                b: [1010.0, 2000.0],
            },
            width: 2.0,
            color: [0.1, 0.2, 0.3, 0.4],
        };
        s.encode(&mut v);
        assert_eq!(v.len(), FLOATS);
        assert_eq!(&v[0..4], &[998.0, 1998.0, 14.0, 4.0]);
        assert_eq!(&v[4..8], &[2.0, 2.0, 12.0, 2.0]);
        assert_eq!(&v[8..12], &[0.0, 0.0, 1.0, 0.0]);
        assert_eq!(&v[12..16], &[0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn round_rect_radius_is_clamped_to_half_the_side() {
        let mut v = Vec::new();
        let s = inst(
            Shape::RoundRect {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 6.0,
                r: 5.0,
            },
            1.0,
        );
        s.encode(&mut v);
        assert_eq!(v[9], 2.0);
    }
}
