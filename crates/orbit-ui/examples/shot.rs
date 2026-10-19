//! Prepares a README screenshot: paints rectangles over regions that must
//! not be published (the bar, the workspace rail) in the brand's midnight,
//! then scales the image to a given width.
//!
//!     cargo run -q --release -p orbit-ui --example shot -- IN OUT \
//!         [--width W] [--mask X,Y,W,H]...
//!
//! Masks are in the input's pixels (device px for a `grim` capture).

use image::{Rgba, imageops::FilterType};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut input, mut output, mut width, mut masks) = (None, None, None, Vec::new());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--width" => {
                width = args.get(i + 1).and_then(|w| w.parse::<u32>().ok());
                i += 1;
            }
            "--mask" => {
                let rect: Vec<u32> = args
                    .get(i + 1)
                    .map(|m| m.split(',').filter_map(|v| v.parse().ok()).collect())
                    .unwrap_or_default();
                if rect.len() == 4 {
                    masks.push((rect[0], rect[1], rect[2], rect[3]));
                } else {
                    eprintln!("--mask wants X,Y,W,H");
                    std::process::exit(2);
                }
                i += 1;
            }
            other if input.is_none() => input = Some(other.to_owned()),
            other if output.is_none() => output = Some(other.to_owned()),
            other => {
                eprintln!("unexpected argument {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let (Some(input), Some(output)) = (input, output) else {
        eprintln!("usage: shot IN OUT [--width W] [--mask X,Y,W,H]...");
        std::process::exit(2);
    };
    let mut img = image::open(&input).expect("readable input").to_rgba8();
    let midnight = Rgba([0x0B, 0x10, 0x20, 0xFF]);
    for (x, y, w, h) in masks {
        for py in y..(y + h).min(img.height()) {
            for px in x..(x + w).min(img.width()) {
                img.put_pixel(px, py, midnight);
            }
        }
    }
    let out = match width {
        Some(w) if w < img.width() => {
            let h = (img.height() as f64 * w as f64 / img.width() as f64).round() as u32;
            image::imageops::resize(&img, w, h, FilterType::Lanczos3)
        }
        _ => img,
    };
    out.save(&output).expect("writable output");
    println!("{output}: {}x{}", out.width(), out.height());
}
