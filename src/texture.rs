use std::io::Cursor;

#[derive(Debug)]
pub struct TextureImage {
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode_png(source: String, encoded: &[u8]) -> Result<TextureImage, String> {
    let mut decoder = png::Decoder::new(Cursor::new(encoded));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("decode PNG {source}: {error}"))?;
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("decode PNG {source}: {error}"))?;
    decoded.truncate(info.buffer_size());

    let pixel_count = (info.width as usize)
        .checked_mul(info.height as usize)
        .ok_or_else(|| format!("PNG dimensions overflow for {source}"))?;
    let mut rgba = Vec::with_capacity(pixel_count.saturating_mul(4));
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(&decoded),
        png::ColorType::Rgb => {
            for pixel in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &gray in &decoded {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in decoded.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err(format!("PNG decoder left indexed pixels in {source}"));
        }
    }
    if rgba.len() != pixel_count.saturating_mul(4) {
        return Err(format!("unexpected decoded PNG size for {source}"));
    }
    Ok(TextureImage {
        source,
        width: info.width,
        height: info.height,
        rgba,
    })
}
