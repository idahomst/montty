//! The window icon: the embedded 256 px PNG decoded to RGBA for winit.

pub(super) const ICON_PNG: &[u8] =
    include_bytes!("../../assets/icon/hicolor/256x256/apps/montty.png");

/// Decodes a PNG to 8-bit RGBA: pixels, width, height.
pub(super) fn decode_rgba(png: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("icon too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "expected RGBA8, got {:?} {:?}",
            info.color_type, info.bit_depth
        ));
    }
    buf.truncate(info.buffer_size());
    Ok((buf, info.width, info.height))
}

/// The window icon, or `None` (after a message on stderr) if it can't be built.
pub(super) fn window_icon() -> Option<winit::window::Icon> {
    let built = decode_rgba(ICON_PNG)
        .and_then(|(px, w, h)| winit::window::Icon::from_rgba(px, w, h).map_err(|e| e.to_string()));
    match built {
        Ok(icon) => Some(icon),
        Err(e) => {
            eprintln!("montty: icon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_decodes_to_256_rgba() {
        let (px, w, h) = decode_rgba(ICON_PNG).unwrap();
        assert_eq!((w, h), (256, 256));
        assert_eq!(px.len(), 256 * 256 * 4);
        assert_eq!(px[3], 0, "rounded corner is transparent");
        let mid = (128 * 256 + 128) * 4;
        assert_eq!(px[mid + 3], 255, "tile centre is opaque");
    }

    #[test]
    fn bad_png_is_an_error_not_a_panic() {
        assert!(decode_rgba(b"not a png").is_err());
    }

    #[test]
    fn window_icon_builds() {
        assert!(window_icon().is_some());
    }
}
