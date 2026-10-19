//! The overview's frosted backdrop (ORB-008): one output capture, taken
//! when the overview opens, downscaled and blurred here so the glass looks
//! the same on every compositor configuration. The capture never touches
//! disk; the blurred image replaces the previous one on each open and is
//! dropped when the overview closes.
//!
//! The blur is three box passes (a close Gaussian) on the downscaled
//! image, run on the capture thread; at a quarter of the output size that
//! is a few milliseconds, and the GPU's bilinear upscale smooths the rest.

use std::time::Instant;

use orbit_capture::Frame;

/// Blur radius in blurred-image pixels per box pass; three passes give a
/// Gaussian of about this sigma: about 42 device px at the desk's
/// downscale of 6 (`orbit_ui::desk::BACKDROP_DOWNSCALE`).
pub const RADIUS: u32 = 7;
const PASSES: u32 = 3;

/// Downscales the frame to `size` and blurs it. BGRA passes through.
pub fn frost(frame: &Frame, size: (u32, u32), radius: u32) -> image::RgbaImage {
    let started = Instant::now();
    let (w, h) = (size.0.max(1), size.1.max(1));
    let mut small = decimate(frame, (w, h));
    let sampled = started.elapsed();
    blur(&mut small, radius.clamp(1, 20), PASSES);
    log::debug!(
        "backdrop: {}x{} -> {w}x{h}, sampled in {:.1} ms, blurred in {:.1} ms",
        frame.width,
        frame.height,
        sampled.as_secs_f64() * 1000.0,
        (started.elapsed() - sampled).as_secs_f64() * 1000.0
    );
    small
}

/// Point-samples the frame down to `size` (the blur that follows removes
/// the aliasing, and reading every source pixel would cost more than the
/// blur itself).
fn decimate(frame: &Frame, size: (u32, u32)) -> image::RgbaImage {
    let (w, h) = (
        size.0.min(frame.width).max(1),
        size.1.min(frame.height).max(1),
    );
    let stride = frame.width as usize * 4;
    image::RgbaImage::from_fn(w, h, |x, y| {
        let sx = (x as u64 * u64::from(frame.width) / u64::from(w)) as usize;
        let sy = (y as u64 * u64::from(frame.height) / u64::from(h)) as usize;
        let at = sy * stride + sx * 4;
        image::Rgba([
            frame.bgra[at],
            frame.bgra[at + 1],
            frame.bgra[at + 2],
            frame.bgra[at + 3],
        ])
    })
}

/// In-place separable box blur, `passes` times, radius `r` (window 2r+1),
/// edges clamped. Alpha is blurred too (it is opaque for captures).
pub fn blur(image: &mut image::RgbaImage, r: u32, passes: u32) {
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w == 0 || h == 0 || r == 0 {
        return;
    }
    let r = r as usize;
    let mut scratch = vec![0u8; w * h * 4];
    for _ in 0..passes {
        box_pass(image.as_raw(), &mut scratch, w, h, r, true);
        box_pass(&scratch, image.as_mut(), w, h, r, false);
    }
}

/// One box pass along rows (`horizontal`) or columns, with a running sum.
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let index = |line: usize, i: usize| {
        if horizontal {
            (line * w + i) * 4
        } else {
            (i * w + line) * 4
        }
    };
    let window = (2 * r + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        // Prime the window with the clamped left edge and the first r.
        for c in 0..4 {
            sum[c] += u32::from(src[index(line, 0) + c]) * (r as u32 + 1);
        }
        for i in 1..=r {
            let at = index(line, i.min(len - 1));
            for c in 0..4 {
                sum[c] += u32::from(src[at + c]);
            }
        }
        for i in 0..len {
            let out = index(line, i);
            for c in 0..4 {
                dst[out + c] = (sum[c] / window) as u8;
            }
            let leaving = index(line, i.saturating_sub(r));
            let entering = index(line, (i + r + 1).min(len - 1));
            for c in 0..4 {
                sum[c] = sum[c] + u32::from(src[entering + c]) - u32::from(src[leaving + c]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_images_stay_uniform_and_edges_soften() {
        let mut flat = image::RgbaImage::from_pixel(40, 30, image::Rgba([10, 20, 30, 255]));
        blur(&mut flat, 4, 3);
        assert!(flat.pixels().all(|p| *p == image::Rgba([10, 20, 30, 255])));

        let mut edge = image::RgbaImage::from_fn(64, 8, |x, _| {
            if x < 32 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        blur(&mut edge, 3, 3);
        let row: Vec<u8> = (0..64).map(|x| edge.get_pixel(x, 4)[0]).collect();
        assert!(
            row.windows(2).all(|p| p[0] <= p[1]),
            "monotonic across the edge"
        );
        assert!(row[0] < 8 && row[63] > 247);
        assert!(
            (100..160).contains(&row[32]),
            "the edge is now a ramp: {}",
            row[32]
        );
    }

    #[test]
    fn frost_downscales_to_the_requested_size() {
        let frame = Frame {
            width: 16,
            height: 8,
            bgra: vec![200; 16 * 8 * 4],
        };
        let out = frost(&frame, (4, 2), RADIUS);
        assert_eq!((out.width(), out.height()), (4, 2));
        assert!(out.pixels().all(|p| p[0] == 200));
    }

    #[test]
    fn blur_at_backdrop_size_is_fast_enough() {
        let mut image = image::RgbaImage::from_fn(480, 300, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let started = Instant::now();
        blur(&mut image, RADIUS, PASSES);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        // The design targets budget 10 ms in release. Debug builds are
        // several times slower, and a cold CI runner (the aarch64 one
        // failed this at 200 ms) slower still, so the debug budget is loose:
        // this guards against a blur that became quadratic, not a slow host.
        let budget = if cfg!(debug_assertions) { 2000.0 } else { 50.0 };
        assert!(ms < budget, "blur took {ms:.1} ms (budget {budget} ms)");
    }
}
