//! Captured pixels, independent of the protocol that produced them. Converts
//! shared-memory formats into the tightly packed BGRA that GPUI uploads, so
//! the UI thread never touches pixel data.

use anyhow::{Result, bail};
use serde::Serialize;
use wayland_client::protocol::wl_shm::Format;

/// Formats we accept from the compositor, best first. All are 32 bits per
/// pixel; the `A`/`X` variants differ only in whether alpha is meaningful.
pub const PREFERRED_FORMATS: [Format; 4] = [
    Format::Argb8888,
    Format::Xrgb8888,
    Format::Abgr8888,
    Format::Xbgr8888,
];

pub fn pick_format(offered: &[Format]) -> Option<Format> {
    PREFERRED_FORMATS
        .into_iter()
        .find(|format| offered.contains(format))
}

/// A tightly packed BGRA8 image (stride == width * 4), premultiplication as
/// the compositor delivered it.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.bgra.len())
            .finish()
    }
}

impl Frame {
    /// Converts a shared-memory buffer. `stride` may exceed `width * 4`.
    /// Little-endian wl_shm formats name channels from the most significant
    /// byte, so ARGB8888 is B, G, R, A in memory.
    pub fn from_shm(
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
        format: Format,
    ) -> Result<Self> {
        let row = width as usize * 4;
        if (stride as usize) < row {
            bail!("stride {stride} is smaller than a {width}px row");
        }
        let needed = stride as usize * height.saturating_sub(1) as usize + row;
        if height > 0 && data.len() < needed {
            bail!("buffer holds {} bytes, need {needed}", data.len());
        }
        let (swap_rb, opaque) = match format {
            Format::Argb8888 => (false, false),
            Format::Xrgb8888 => (false, true),
            Format::Abgr8888 => (true, false),
            Format::Xbgr8888 => (true, true),
            other => bail!("unsupported shm format {other:?}"),
        };
        let mut bgra = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let start = y * stride as usize;
            bgra.extend_from_slice(&data[start..start + row]);
        }
        if swap_rb || opaque {
            for pixel in bgra.as_chunks_mut::<4>().0 {
                if swap_rb {
                    pixel.swap(0, 2);
                }
                if opaque {
                    pixel[3] = 0xff;
                }
            }
        }
        Ok(Self {
            width,
            height,
            bgra,
        })
    }

    pub fn bytes(&self) -> usize {
        self.bgra.len()
    }

    /// The same pixels as RGBA, for encoding to PNG.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = self.bgra.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        rgba
    }

    pub fn stats(&self) -> Stats {
        let pixels = (self.width as usize * self.height as usize).max(1);
        let mut lit = 0usize;
        let mut transparent = 0usize;
        let mut sum = [0u64; 3];
        for p in self.bgra.as_chunks::<4>().0 {
            if p[0] > 8 || p[1] > 8 || p[2] > 8 {
                lit += 1;
            }
            if p[3] == 0 {
                transparent += 1;
            }
            sum[0] += u64::from(p[2]);
            sum[1] += u64::from(p[1]);
            sum[2] += u64::from(p[0]);
        }
        let mean = sum.map(|s| (s / pixels as u64) as u8);
        Stats {
            lit_fraction: lit as f64 / pixels as f64,
            transparent_fraction: transparent as f64 / pixels as f64,
            mean_rgb: format!("#{:02x}{:02x}{:02x}", mean[0], mean[1], mean[2]),
        }
    }

    /// Fraction of pixels within `tolerance` per channel of `rgb` (0xRRGGBB).
    pub fn color_fraction(&self, rgb: u32, tolerance: u8) -> f64 {
        let target = [
            (rgb & 0xff) as u8,
            (rgb >> 8 & 0xff) as u8,
            (rgb >> 16) as u8,
        ];
        let pixels = (self.width as usize * self.height as usize).max(1);
        let hits = self
            .bgra
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| (0..3).all(|i| p[i].abs_diff(target[i]) <= tolerance))
            .count();
        hits as f64 / pixels as f64
    }
}

/// Cheap content checks that catch black, empty or garbled captures.
#[derive(Clone, Debug, Serialize)]
pub struct Stats {
    /// Pixels that are not (near) black.
    pub lit_fraction: f64,
    pub transparent_fraction: f64,
    pub mean_rgb: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_argb_then_xrgb() {
        assert_eq!(
            pick_format(&[Format::Xbgr8888, Format::Xrgb8888]),
            Some(Format::Xrgb8888)
        );
        assert_eq!(pick_format(&[Format::Rgb565]), None);
    }

    #[test]
    fn strips_stride_padding_and_forces_opaque_alpha() {
        // 2x2 XRGB with 4 bytes of padding per row; X bytes are garbage.
        let data = [
            1, 2, 3, 0, 4, 5, 6, 9, 0xee, 0xee, 0xee, 0xee, //
            7, 8, 9, 0, 10, 11, 12, 0, 0xee, 0xee, 0xee, 0xee,
        ];
        let frame = Frame::from_shm(&data, 2, 2, 12, Format::Xrgb8888).unwrap();
        assert_eq!(
            frame.bgra,
            [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
    }

    #[test]
    fn swaps_abgr_into_bgra() {
        // ABGR8888 little-endian is R, G, B, A in memory.
        let frame = Frame::from_shm(&[10, 20, 30, 40], 1, 1, 4, Format::Abgr8888).unwrap();
        assert_eq!(frame.bgra, [30, 20, 10, 40]);
        assert_eq!(frame.to_rgba(), [10, 20, 30, 40]);
    }

    #[test]
    fn rejects_short_buffers_and_bad_strides() {
        assert!(Frame::from_shm(&[0; 7], 1, 2, 4, Format::Argb8888).is_err());
        assert!(Frame::from_shm(&[0; 16], 2, 2, 4, Format::Argb8888).is_err());
        assert!(Frame::from_shm(&[0; 4], 1, 1, 4, Format::Rgb565).is_err());
    }

    #[test]
    fn last_row_needs_no_padding() {
        let data = [0u8; 12 + 8];
        assert!(Frame::from_shm(&data, 2, 2, 12, Format::Argb8888).is_ok());
    }

    #[test]
    fn stats_and_color_match() {
        let frame = Frame {
            width: 2,
            height: 1,
            bgra: vec![0x20, 0x10, 0x0b, 255, 0, 0, 0, 0],
        };
        let stats = frame.stats();
        assert_eq!(stats.lit_fraction, 0.5);
        assert_eq!(stats.transparent_fraction, 0.5);
        assert_eq!(frame.color_fraction(0x0b1020, 0), 0.5);
    }
}
