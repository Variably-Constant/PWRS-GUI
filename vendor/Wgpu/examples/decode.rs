//! The Rust library decoding image files with no device: each frame's size
//! and the formats of its pixels as the file stores them and as
//! Import-WgpuImage loads them, what decoding did, and an option it
//! refuses. Run it with
//!
//!     cargo run --example decode --no-default-features --features svg -- tests/svg/shapes.svg tests/ps-render/fills-gray.psd

use wgpu_pwsh::{DecodeOptions, decode_image};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    for path in &paths {
        let bytes = std::fs::read(path)?;
        let image = decode_image(&bytes, Some(path), &DecodeOptions::default())?;
        for frame in &image.frames {
            println!("{path}: {}x{}, stored as {:?}, displayed as {:?}", frame.width, frame.height, frame.stored.format, frame.display.format);
        }
        for note in &image.notes {
            println!("  {note}");
        }
        for warning in &image.warnings {
            println!("  warning: {warning}");
        }
    }
    if let Some(path) = paths.first() {
        let options = DecodeOptions { frames: Some(vec![3]), ..Default::default() };
        match decode_image(&std::fs::read(path)?, Some(path), &options) {
            Ok(image) => println!("{path}: frame 3 of {} decoded", image.frames.len()),
            Err(e) => println!("refused as {}: {}", e.error_id, e.message),
        }
    }
    Ok(())
}
