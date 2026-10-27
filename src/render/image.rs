//! Background images: decoding (PNG, JPEG), downscaling, the fit math, and the textured
//! quad that draws one behind a pane's text.

use glow::HasContext;

use crate::config::profile::Fit;
use crate::layout::Rect;

/// Images with more pixels than this are refused before decoding (512 MiB of RGBA8).
const MAX_PIXELS: u64 = 1 << 27;

/// An image as 8-bit RGBA, straight (not premultiplied) alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba8 {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// A PNG (any color type and depth) or a JPEG, told apart by their magic bytes, as RGBA8.
pub fn decode(bytes: &[u8]) -> Result<Rgba8, String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        decode_jpeg(bytes)
    } else {
        Err("unsupported image format".into())
    }
}

/// The RGBA8 byte length of a `w` × `h` image, unless it is empty or too large.
fn rgba_len(w: u32, h: u32) -> Result<usize, String> {
    let n = u64::from(w) * u64::from(h);
    if n == 0 {
        return Err("empty image".into());
    }
    if n > MAX_PIXELS {
        return Err(format!("image too large ({w}×{h})"));
    }
    usize::try_from(n * 4).map_err(|_| format!("image too large ({w}×{h})"))
}

fn decode_png(bytes: &[u8]) -> Result<Rgba8, String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes and tRNS expand to alpha, low depths to 8 bits, 16 bits strip to 8: the
    // output is gray+alpha or RGBA, 8 bits each.
    d.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    // The size is checked from IHDR alone, before reading on or allocating anything.
    let (w, h) = d.read_header_info().map_err(|e| e.to_string())?.size();
    let len = rgba_len(w, h)?;
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    if r.output_buffer_size().is_none_or(|n| n > len) {
        return Err("unexpected PNG layout".into());
    }
    let mut px = vec![0; len];
    let info = r.next_frame(&mut px).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    if info.bit_depth != png::BitDepth::Eight || channels == 0 {
        return Err("unexpected PNG layout".into());
    }
    expand_in_place(&mut px, channels);
    Ok(Rgba8 { w, h, px })
}

/// Widen `channels`-per-pixel 8-bit samples (gray, gray+alpha, RGB) packed at the start of
/// `px` to RGBA over the whole buffer, back to front so nothing is overwritten before it is
/// read. RGBA is left as is.
fn expand_in_place(px: &mut [u8], channels: usize) {
    let n = px.len() / 4;
    for i in (0..n).rev() {
        let s = i * channels;
        let rgba = match channels {
            1 => [px[s], px[s], px[s], 255],
            2 => [px[s], px[s], px[s], px[s + 1]],
            3 => [px[s], px[s + 1], px[s + 2], 255],
            _ => return,
        };
        px[i * 4..i * 4 + 4].copy_from_slice(&rgba);
    }
}

fn decode_jpeg(bytes: &[u8]) -> Result<Rgba8, String> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    // JPEG's own size limit; MAX_PIXELS is checked below.
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX));
    let mut d = JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers().map_err(|e| e.to_string())?;
    let info = d.info().ok_or("no JPEG header")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let len = rgba_len(w, h)?;
    if d.output_buffer_size() != Some(len) {
        return Err("unexpected JPEG layout".into());
    }
    let mut px = vec![0; len];
    d.decode_into(&mut px).map_err(|e| e.to_string())?;
    Ok(Rgba8 { w, h, px })
}

/// `img` shrunk to fit in `max_w` × `max_h`, keeping its aspect ratio, by a box filter
/// (each output pixel averages the source pixels it covers, weighted by alpha so the color
/// of transparent pixels doesn't bleed in). An image that already fits is returned as is.
pub fn downscale(img: Rgba8, max_w: u32, max_h: u32) -> Rgba8 {
    if (img.w <= max_w && img.h <= max_h) || img.w == 0 || img.h == 0 {
        return img;
    }
    let s = (f64::from(max_w) / f64::from(img.w)).min(f64::from(max_h) / f64::from(img.h));
    let nw = ((f64::from(img.w) * s).round() as u32).clamp(1, max_w.max(1));
    let nh = ((f64::from(img.h) * s).round() as u32).clamp(1, max_h.max(1));
    // Source span [start, end) of each output column or row; never empty, as n ≤ len.
    let spans = |len: u32, n: u32| -> Vec<(usize, usize)> {
        let at = |i: u32| (u64::from(i) * u64::from(len) / u64::from(n)) as usize;
        (0..n).map(|i| (at(i), at(i + 1).max(at(i) + 1))).collect()
    };
    let (cols, rows) = (spans(img.w, nw), spans(img.h, nh));
    let w = img.w as usize;
    let mut out = Vec::with_capacity(nw as usize * nh as usize * 4);
    // Per source column of the current output row: Σ r·a, Σ g·a, Σ b·a, Σ a.
    let mut acc = vec![0u64; w * 4];
    for &(y0, y1) in &rows {
        acc.fill(0);
        for row in img.px[y0 * w * 4..y1 * w * 4].chunks_exact(w * 4) {
            for (sum, p) in acc.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                let a = u64::from(p[3]);
                sum[0] += u64::from(p[0]) * a;
                sum[1] += u64::from(p[1]) * a;
                sum[2] += u64::from(p[2]) * a;
                sum[3] += a;
            }
        }
        for &(x0, x1) in &cols {
            let mut s = [0u64; 4];
            for sum in acc[x0 * 4..x1 * 4].chunks_exact(4) {
                for c in 0..4 {
                    s[c] += sum[c];
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as u64;
            let div = |num: u64, den: u64| {
                if den == 0 {
                    0
                } else {
                    ((num + den / 2) / den) as u8
                }
            };
            out.extend_from_slice(&[
                div(s[0], s[3]),
                div(s[1], s[3]),
                div(s[2], s[3]),
                div(s[3], n),
            ]);
        }
    }
    Rgba8 {
        w: nw,
        h: nh,
        px: out,
    }
}

/// Where an `img` (w, h) image goes in `area` for `fit`: the destination rect and the uv
/// (x0, y0, x1, y1) of the image shown there. One rect for every mode; Tile's uv counts
/// tiles from the area's top-left (beyond 1, see `repeats`), so the texture's repeat wrap
/// draws them, edge tiles cropped. None when there is nothing to draw.
pub fn fit_rect(fit: Fit, img: (u32, u32), area: Rect) -> Option<(Rect, [f32; 4])> {
    let (iw, ih) = img;
    if area.w <= 0 || area.h <= 0 || iw == 0 || ih == 0 {
        return None;
    }
    let (aw, ah) = (area.w as f32, area.h as f32);
    let (fw, fh) = (iw as f32, ih as f32);
    Some(match fit {
        Fit::Stretch => (area, [0.0, 0.0, 1.0, 1.0]),
        Fit::Fill => {
            // Scaled to cover; the visible fraction of each axis is centered.
            let s = (aw / fw).max(ah / fh);
            let (vx, vy) = (aw / (fw * s), ah / (fh * s));
            let (u0, v0) = ((1.0 - vx) / 2.0, (1.0 - vy) / 2.0);
            (area, [u0, v0, u0 + vx, v0 + vy])
        }
        Fit::Center => {
            let (x, w, u0, u1) = center_axis(iw, area.x, area.w);
            let (y, h, v0, v1) = center_axis(ih, area.y, area.h);
            (Rect { x, y, w, h }, [u0, v0, u1, v1])
        }
        Fit::Tile => (area, [0.0, 0.0, aw / fw, ah / fh]),
    })
}

/// Whether `fit`'s uv goes beyond 0..1 and the texture must repeat.
pub fn repeats(fit: Fit) -> bool {
    fit == Fit::Tile
}

/// One axis of Center: an image `len` px long centered in `area_len` px at `area_pos`; the
/// position, the length shown and the uv range, cropped evenly when it is longer.
fn center_axis(len: u32, area_pos: i32, area_len: i32) -> (i32, i32, f32, f32) {
    let n = len.min(i32::MAX as u32) as i32;
    if n <= area_len {
        return (area_pos + (area_len - n) / 2, n, 0.0, 1.0);
    }
    let off = (n - area_len) / 2;
    let f = len as f32;
    (
        area_pos,
        area_len,
        off as f32 / f,
        (off + area_len) as f32 / f,
    )
}

/// Straight RGBA8 to premultiplied in place: rgb·a/255, rounded. Linear filtering of
/// premultiplied texels doesn't pull the color of transparent texels into the edges.
pub(crate) fn premultiply(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// An uploaded image: a premultiplied RGBA8 texture with mipmaps, filtered linearly.
#[derive(Debug)]
pub struct ImageTexture {
    tex: glow::Texture,
    pub w: u32,
    pub h: u32,
}

/// Draws an `ImageTexture` into a rect with a uv range and an alpha.
pub(crate) struct ImagePipeline {
    program: glow::Program,
    vao: glow::VertexArray,
    u_proj: Option<glow::UniformLocation>,
    u_rect: Option<glow::UniformLocation>,
    u_uv: Option<glow::UniformLocation>,
    u_alpha: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
}

impl ImagePipeline {
    pub(crate) fn new(gl: &glow::Context, quad: glow::Buffer) -> Result<Self, String> {
        let program = super::program(
            gl,
            include_str!("shaders/image.vert"),
            include_str!("shaders/image.frag"),
        )?;
        unsafe {
            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            super::bind_quad(gl, quad);
            gl.bind_vertex_array(None);
            let loc = |name| gl.get_uniform_location(program, name);
            Ok(ImagePipeline {
                u_proj: loc("u_proj"),
                u_rect: loc("u_rect"),
                u_uv: loc("u_uv"),
                u_alpha: loc("u_alpha"),
                u_tex: loc("u_tex"),
                program,
                vao,
            })
        }
    }

    /// Upload `img` as a new texture with mipmaps, so an image shown smaller than it is
    /// doesn't alias. Shown at its own size (Center, Tile) it samples level 0 exactly. A GL
    /// error (out of memory) is an Err, and nothing is kept. `img` stays straight alpha: a
    /// premultiplied copy is uploaded, once, before the mipmaps are built.
    pub(crate) fn upload(gl: &glow::Context, img: &Rgba8) -> Result<ImageTexture, String> {
        unsafe {
            // Errors from before aren't this upload's.
            for _ in 0..8 {
                if gl.get_error() == glow::NO_ERROR {
                    break;
                }
            }
            let mut px = img.px.clone();
            premultiply(&mut px);
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                img.w as i32,
                img.h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&px)),
            );
            gl.generate_mipmap(glow::TEXTURE_2D);
            let err = gl.get_error();
            if err != glow::NO_ERROR {
                gl.delete_texture(tex);
                return Err(format!("can't upload the image (GL error 0x{err:x})"));
            }
            Ok(ImageTexture {
                tex,
                w: img.w,
                h: img.h,
            })
        }
    }

    pub(crate) fn delete(gl: &glow::Context, t: ImageTexture) {
        unsafe { gl.delete_texture(t.tex) };
    }

    /// Draw `t`'s `uv` range into `dest` at `alpha`; `repeat` wraps a uv beyond 0..1 (Tile).
    // One quad's uniforms; bundling them would only rename the argument list.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        gl: &glow::Context,
        proj: &[f32; 16],
        t: &ImageTexture,
        dest: Rect,
        uv: [f32; 4],
        alpha: f32,
        repeat: bool,
    ) {
        let wrap = if repeat {
            glow::REPEAT
        } else {
            glow::CLAMP_TO_EDGE
        } as i32;
        let r = [dest.x as f32, dest.y as f32, dest.w as f32, dest.h as f32];
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_proj.as_ref(), false, proj);
            gl.uniform_4_f32_slice(self.u_rect.as_ref(), &r);
            gl.uniform_4_f32_slice(self.u_uv.as_ref(), &uv);
            gl.uniform_1_f32(self.u_alpha.as_ref(), alpha);
            gl.uniform_1_i32(self.u_tex.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 200,
        h: 100,
    };

    fn png_bytes(
        (w, h): (u32, u32),
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
        palette: Option<(&[u8], &[u8])>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(depth);
        if let Some((plte, trns)) = palette {
            e.set_palette(plte.to_vec());
            e.set_trns(trns.to_vec());
        }
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(data).unwrap();
        wr.finish().unwrap();
        out
    }

    fn px(img: &Rgba8, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.w + x) * 4) as usize;
        img.px[i..i + 4].try_into().unwrap()
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 8)
    }

    #[test]
    fn decode_png_and_jpeg() {
        use png::{BitDepth, ColorType};
        // RGB8, 3×2: pixel (2,1) is (10,20,30).
        let mut rgb = vec![200u8; 3 * 2 * 3];
        rgb[15..18].copy_from_slice(&[10, 20, 30]);
        let img = decode(&png_bytes(
            (3, 2),
            ColorType::Rgb,
            BitDepth::Eight,
            &rgb,
            None,
        ))
        .unwrap();
        assert_eq!((img.w, img.h, img.px.len()), (3, 2, 3 * 2 * 4));
        assert_eq!(px(&img, 2, 1), [10, 20, 30, 255]);
        assert_eq!(px(&img, 0, 0), [200, 200, 200, 255]);

        // Every other color type and depth comes out as RGBA8 too.
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let img = decode(&png_bytes(
            (2, 1),
            ColorType::Rgba,
            BitDepth::Eight,
            &rgba,
            None,
        ))
        .unwrap();
        assert_eq!(img.px, rgba);
        // 16-bit RGB: the high bytes.
        let rgb16 = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
        let img = decode(&png_bytes(
            (1, 1),
            ColorType::Rgb,
            BitDepth::Sixteen,
            &rgb16,
            None,
        ))
        .unwrap();
        assert_eq!(img.px, [0x12, 0x56, 0x9a, 255]);
        // 4-bit gray: 0xf0 is a white pixel then a black one.
        let img = decode(&png_bytes(
            (2, 1),
            ColorType::Grayscale,
            BitDepth::Four,
            &[0xf0],
            None,
        ))
        .unwrap();
        assert_eq!(img.px, [255, 255, 255, 255, 0, 0, 0, 255]);
        let ga = [100, 50];
        let img = decode(&png_bytes(
            (1, 1),
            ColorType::GrayscaleAlpha,
            BitDepth::Eight,
            &ga,
            None,
        ))
        .unwrap();
        assert_eq!(img.px, [100, 100, 100, 50]);
        // Indexed with a transparent entry.
        let plte = [255, 0, 0, 0, 0, 255];
        let img = decode(&png_bytes(
            (2, 1),
            ColorType::Indexed,
            BitDepth::Eight,
            &[0, 1],
            Some((&plte, &[255, 0])),
        ))
        .unwrap();
        assert_eq!(img.px, [255, 0, 0, 255, 0, 0, 255, 0]);

        // JPEG, 16×8 of one color.
        let color = [200u8, 100, 50];
        let data: Vec<u8> = color.iter().copied().cycle().take(16 * 8 * 3).collect();
        let mut jpg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpg, 95)
            .encode(&data, 16, 8, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let img = decode(&jpg).unwrap();
        assert_eq!((img.w, img.h, img.px.len()), (16, 8, 16 * 8 * 4));
        assert!(
            near(px(&img, 5, 3), [200, 100, 50, 255]),
            "{:?}",
            px(&img, 5, 3)
        );
        // A grayscale JPEG.
        let mut jpg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpg, 95)
            .encode(&[90; 8 * 8], 8, 8, jpeg_encoder::ColorType::Luma)
            .unwrap();
        let img = decode(&jpg).unwrap();
        assert_eq!((img.w, img.h), (8, 8));
        assert!(
            near(px(&img, 4, 4), [90, 90, 90, 255]),
            "{:?}",
            px(&img, 4, 4)
        );
    }

    #[test]
    fn decode_rejects_corrupt() {
        let rgb = vec![7u8; 64 * 64 * 3];
        let full = png_bytes(
            (64, 64),
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &rgb,
            None,
        );
        assert!(decode(&full).is_ok());
        assert!(decode(&full[..full.len() / 2]).is_err(), "truncated PNG");
        assert!(decode(&full[..20]).is_err(), "header only");
        let mut jpg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpg, 90)
            .encode(&rgb, 64, 64, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        assert!(decode(&jpg[..40]).is_err(), "truncated JPEG");
        let e = decode(b"GIF89a\x01\x00\x01\x00\x00\x00\x00").unwrap_err();
        assert!(e.contains("unsupported"), "{e}");
        assert!(decode(b"").unwrap_err().contains("unsupported"));
    }

    /// A PNG chunk: length, type, data, CRC-32 of type and data.
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut crc = !0u32;
        for &b in kind.iter().chain(data) {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&(!crc).to_be_bytes());
        out
    }

    #[test]
    fn decode_rejects_bomb() {
        // A valid IHDR declaring 100000×100000 RGB8 (30 GB of RGB), then a tiny IDAT: the
        // size is refused from the header, before anything that size is allocated.
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &ihdr));
        // The check needs nothing after the header: no IDAT at all, or a tiny one.
        let e = decode(&png).unwrap_err();
        assert!(e.contains("too large"), "{e}");
        png.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00]));
        png.extend(chunk(b"IEND", &[]));
        let e = decode(&png).unwrap_err();
        assert!(e.contains("too large"), "{e}");
        // The fixture is a well-formed PNG header: the same bytes at 2×2 get past it.
        let mut ok = b"\x89PNG\r\n\x1a\n".to_vec();
        ok.extend(chunk(b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 2, 8, 2, 0, 0, 0]));
        let e = decode(&ok).unwrap_err();
        assert!(!e.contains("too large"), "{e}");
    }

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Rgba8 {
        Rgba8 {
            w,
            h,
            px: c.repeat((w * h) as usize),
        }
    }

    #[test]
    fn downscale_bounds() {
        let img = downscale(solid(8000, 1000, [10, 20, 30, 255]), 3840, 2160);
        assert_eq!((img.w, img.h, img.px.len()), (3840, 480, 3840 * 480 * 4));
        assert_eq!(px(&img, 3839, 479), [10, 20, 30, 255]);
        let small = solid(100, 100, [1, 2, 3, 4]);
        assert_eq!(downscale(small.clone(), 3840, 2160), small);
        // Tall: height-bound.
        let img = downscale(solid(1000, 8000, [0; 4]), 3840, 2160);
        assert_eq!((img.w, img.h), (270, 2160));
        // A box filter averages: a 2×2 checker of black and white into 1×1 is mid gray.
        let checker = Rgba8 {
            w: 2,
            h: 2,
            px: [[0, 0, 0, 255], [255; 4], [255; 4], [0, 0, 0, 255]].concat(),
        };
        let one = downscale(checker, 1, 1);
        assert!(near(px(&one, 0, 0), [128, 128, 128, 255]), "{:?}", one.px);
        // Transparent pixels don't bleed their color: weighted by alpha.
        let edge = Rgba8 {
            w: 2,
            h: 1,
            px: [[255, 0, 0, 255], [0, 0, 255, 0]].concat(),
        };
        let one = downscale(edge, 1, 1);
        assert!(near(px(&one, 0, 0), [255, 0, 0, 128]), "{:?}", one.px);
    }

    #[test]
    fn premultiply_edges() {
        let mut p = [200, 100, 50, 0, 200, 100, 50, 255, 200, 100, 50, 128];
        premultiply(&mut p);
        assert_eq!(p, [0, 0, 0, 0, 200, 100, 50, 255, 100, 50, 25, 128]);
    }

    #[test]
    fn downscale_of_premultiplied_keeps_color_at_edges() {
        // Opaque red next to transparent black, premultiplied, then averaged as linear
        // filtering does: half red at half alpha, which un-premultiplies to pure red.
        let mut p = [255, 0, 0, 255, 0, 0, 0, 0];
        premultiply(&mut p);
        let avg: Vec<u32> = (0..4)
            .map(|i| (u32::from(p[i]) + u32::from(p[4 + i])).div_ceil(2))
            .collect();
        let a = avg[3];
        assert!(a.abs_diff(128) <= 1, "{avg:?}");
        let r = avg[0] * 255 / a;
        assert!(r >= 253, "red {r}, not dark red: {avg:?}");
        assert_eq!((avg[1], avg[2]), (0, 0));
    }

    fn uv_near(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    /// `fit_rect` as a list (empty or one rect).
    fn rects(fit: Fit, img: (u32, u32), area: Rect) -> Vec<(Rect, [f32; 4])> {
        super::fit_rect(fit, img, area).into_iter().collect()
    }

    #[test]
    fn fit_rects() {
        let full = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(rects(Fit::Stretch, (64, 64), AREA), vec![(AREA, full)]);

        // Fill: a square covers 200×100 by cropping the middle half vertically.
        let r = rects(Fit::Fill, (100, 100), AREA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, AREA);
        assert!(uv_near(r[0].1, [0.0, 0.25, 1.0, 0.75]), "{:?}", r[0].1);

        // Center: natural size, centered.
        assert_eq!(
            rects(Fit::Center, (50, 50), AREA),
            vec![(
                Rect {
                    x: 75,
                    y: 25,
                    w: 50,
                    h: 50
                },
                full
            )]
        );
        // Larger than the area: cropped through the uv, centered.
        let r = rects(Fit::Center, (400, 50), AREA);
        assert_eq!(
            r[0].0,
            Rect {
                x: 0,
                y: 25,
                w: 200,
                h: 50
            }
        );
        assert!(uv_near(r[0].1, [0.25, 0.0, 0.75, 1.0]), "{:?}", r[0].1);

        // Tile: one rect over the area, the uv counting tiles (the texture repeats), so
        // 64×64 tiles from the top-left make 4×2 tiles, the last one cropped to 8×36.
        let r = rects(Fit::Tile, (64, 64), AREA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, AREA);
        assert!(
            uv_near(r[0].1, [0.0, 0.0, 200.0 / 64.0, 100.0 / 64.0]),
            "{:?}",
            r[0].1
        );
        let (u1, v1) = (r[0].1[2], r[0].1[3]);
        assert_eq!((u1.ceil(), v1.ceil()), (4.0, 2.0), "4×2 tiles");
        assert!(((u1.fract() * 64.0) - 8.0).abs() < 1e-3, "last column 8 px");
        assert!(((v1.fract() * 64.0) - 36.0).abs() < 1e-3, "last row 36 px");

        // Offset areas keep their origin.
        let off = Rect {
            x: 10,
            y: 30,
            ..AREA
        };
        assert_eq!(rects(Fit::Stretch, (8, 8), off)[0].0, off);
        // Nothing to draw: an empty area or image.
        let empty = Rect { w: 0, ..AREA };
        for fit in [Fit::Stretch, Fit::Fill, Fit::Center, Fit::Tile] {
            assert!(rects(fit, (64, 64), empty).is_empty());
            assert!(rects(fit, (0, 64), AREA).is_empty());
        }
    }

    #[test]
    fn tile_of_tiny_image_is_one_rect() {
        // A 1×1 image over a 4K pane would be 8 million tiles; it is one repeating quad.
        let area = Rect {
            x: 0,
            y: 0,
            w: 3840,
            h: 2160,
        };
        let r = rects(Fit::Tile, (1, 1), area);
        assert_eq!(r, vec![(area, [0.0, 0.0, 3840.0, 2160.0])]);
        assert!(repeats(Fit::Tile));
        for fit in [Fit::Stretch, Fit::Fill, Fit::Center] {
            assert!(!repeats(fit));
        }
    }
}
