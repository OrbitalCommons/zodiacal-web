//! Decode uploaded image bytes (FITS, JPEG, PNG, TIFF) into an ndarray for plate solving.

use anyhow::{bail, Context, Result};
use fitsio_pure::hdu::parse_fits;
use fitsio_pure::image::{image_dimensions, read_image_data, ImageData};
use ndarray::Array2;

/// Decode raw file bytes into a grayscale f32 image array.
///
/// The file format is detected from the leading bytes:
/// - FITS files start with `SIMPLE  =`
/// - Otherwise, the `image` crate handles JPEG, PNG, and TIFF.
pub fn decode_image(bytes: &[u8]) -> Result<Array2<f32>> {
    if is_fits(bytes) {
        decode_fits(bytes)
    } else {
        decode_raster(bytes)
    }
}

/// Check whether bytes look like a FITS file (magic: "SIMPLE  =").
fn is_fits(bytes: &[u8]) -> bool {
    bytes.len() >= 9 && &bytes[..9] == b"SIMPLE  ="
}

/// Decode a FITS file from bytes using fitsio-pure.
fn decode_fits(bytes: &[u8]) -> Result<Array2<f32>> {
    let fits = parse_fits(bytes).context("Failed to parse FITS")?;
    let hdu = fits.primary();

    let dims = image_dimensions(hdu).context("Failed to read FITS dimensions")?;
    if dims.len() < 2 {
        bail!("FITS image must be at least 2D, got {}D", dims.len());
    }

    // FITS NAXIS ordering: NAXIS1=width, NAXIS2=height (column-major convention),
    // but fitsio-pure returns data in row-major order matching the byte stream.
    let (height, width) = if dims.len() == 2 {
        (dims[0], dims[1])
    } else {
        // For cubes, take the first plane (dims[0] is depth, dims[1]=height, dims[2]=width)
        (dims[dims.len() - 2], dims[dims.len() - 1])
    };

    let img_data = read_image_data(bytes, hdu).context("Failed to read FITS image data")?;

    let pixels = match img_data {
        ImageData::F32(v) => v,
        ImageData::F64(v) => v.iter().map(|&x| x as f32).collect(),
        ImageData::I16(v) => v.iter().map(|&x| x as f32).collect(),
        ImageData::I32(v) => v.iter().map(|&x| x as f32).collect(),
        ImageData::I64(v) => v.iter().map(|&x| x as f32).collect(),
        ImageData::U8(v) => v.iter().map(|&x| x as f32).collect(),
    };

    // For cubes, only use the first plane
    let plane_size = height * width;
    let plane_pixels = if pixels.len() > plane_size {
        pixels[..plane_size].to_vec()
    } else {
        pixels
    };

    Array2::from_shape_vec((height, width), plane_pixels)
        .context("FITS pixel count does not match dimensions")
}

/// Decode JPEG, PNG, or TIFF using the `image` crate.
fn decode_raster(bytes: &[u8]) -> Result<Array2<f32>> {
    let img = image::load_from_memory(bytes).context("Failed to decode image")?;
    let gray = img.to_luma32f();
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    let pixels = gray.into_raw();
    Array2::from_shape_vec((height, width), pixels)
        .context("Pixel count does not match image dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fits_detects_magic() {
        let fits_header = b"SIMPLE  = T / Standard FITS";
        assert!(is_fits(fits_header));
    }

    #[test]
    fn is_fits_rejects_jpeg() {
        let jpeg_header = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49];
        assert!(!is_fits(&jpeg_header));
    }

    #[test]
    fn is_fits_rejects_short() {
        assert!(!is_fits(b"SHORT"));
    }

    #[test]
    fn decode_raster_png() {
        // Create a minimal 2x2 grayscale PNG in memory
        use image::ImageEncoder;
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        let pixels: Vec<u8> = vec![0, 128, 255, 64];
        encoder
            .write_image(&pixels, 2, 2, image::ExtendedColorType::L8)
            .unwrap();
        let result = decode_image(&buf).unwrap();
        assert_eq!(result.shape(), &[2, 2]);
    }
}
