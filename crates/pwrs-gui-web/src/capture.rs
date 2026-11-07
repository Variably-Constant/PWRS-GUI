//! The pixels a webview shows, through each engine's own capture: WebView2's
//! `CapturePreview` on Windows, WebKitGTK's snapshot on Linux and FreeBSD,
//! and WebKit's `takeSnapshotWithConfiguration` on macOS. Each finishes on
//! the event loop's thread after the call that started it has returned.

use pwrs_gui_proto::Image;
use wry::WebView;

/// Runs once, on the event loop's thread, with the capture or why there is
/// none.
pub(crate) type Done = Box<dyn FnOnce(Result<Image, String>)>;

/// A rectangle in the page's CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The part of `image` that `rect` covers, where `viewport` is the page's
/// visible size in the same CSS pixels as `rect`; edges round to the
/// nearest device pixel.
pub(crate) fn crop(image: &Image, rect: Rect, viewport: (f64, f64)) -> Result<Image, String> {
    let (vw, vh) = viewport;
    if !(vw > 0.0 && vh > 0.0) {
        return Err(format!("the page reported a viewport of {vw} by {vh}"));
    }
    let (sx, sy) = (f64::from(image.width) / vw, f64::from(image.height) / vh);
    let px = |v: f64, scale: f64, max: u32| -> u32 {
        let scaled = (v * scale).round().max(0.0);
        if scaled >= f64::from(max) { max } else { scaled as u32 }
    };
    let (x0, x1) = (px(rect.x, sx, image.width), px(rect.x + rect.width, sx, image.width));
    let (y0, y1) = (px(rect.y, sy, image.height), px(rect.y + rect.height, sy, image.height));
    if x1 <= x0 || y1 <= y0 {
        let Rect { x, y, width, height } = rect;
        return Err(format!(
            "the widget, {width} by {height} at ({x}, {y}) in a page {vw} by {vh}, lies outside the {}x{} screenshot",
            image.width, image.height
        ));
    }
    let stride = image.width as usize * 4;
    let row = (x1 - x0) as usize * 4;
    let mut rgba = Vec::with_capacity(row * (y1 - y0) as usize);
    for y in y0..y1 {
        let start = y as usize * stride + x0 as usize * 4;
        let line = image.rgba.get(start..start + row).ok_or_else(|| format!("a {}x{} capture has {} bytes", image.width, image.height, image.rgba.len()))?;
        rgba.extend_from_slice(line);
    }
    Ok(Image { width: x1 - x0, height: y1 - y0, rgba })
}

/// A PNG's pixels as RGBA with 8 bits a channel, rows top to bottom.
#[cfg(any(windows, target_os = "macos"))]
fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("the capture is not a PNG this renderer reads: {e}"))?;
    let size = reader.output_buffer_size().ok_or("the capture is too large to fit in memory")?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("the capture's PNG could not be decoded: {e}"))?;
    let pixels = (info.width as usize) * (info.height as usize);
    let samples = buf.get(..info.line_size * info.height as usize).ok_or("the capture's PNG is shorter than its header says")?;
    // A ragged tail left by as_chunks fails the length check below.
    let rgba = match info.color_type {
        png::ColorType::Rgba => samples.to_vec(),
        png::ColorType::Rgb => samples.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => samples.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Grayscale => samples.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(String::from("the capture's PNG still has its palette after expansion")),
    };
    if rgba.len() != pixels * 4 {
        return Err(format!("the capture's PNG decoded to {} bytes for {}x{} pixels", rgba.len(), info.width, info.height));
    }
    Ok(Image { width: info.width, height: info.height, rgba })
}

/// Starts WebView2's capture of what `webview` shows, as a PNG in a memory
/// stream; `done` runs from WebView2's completion handler.
#[cfg(windows)]
pub(crate) fn start(webview: &WebView, done: Done) {
    use std::cell::Cell;
    use std::rc::Rc;
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
    use wry::WebViewExtWindows;

    // SAFETY: a null HGLOBAL requests a new block, which the stream frees
    // when it is released.
    let stream = match unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) } {
        Ok(s) => s,
        Err(e) => return done(Err(format!("no memory stream for the capture could be made: {e}"))),
    };
    let slot = Rc::new(Cell::new(Some(done)));
    let answer = Rc::clone(&slot);
    let filled = stream.clone();
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        if let Some(done) = answer.take() {
            let image = result.map_err(|e| format!("WebView2 could not capture the page: {e}")).and_then(|()| read_stream(&filled)).and_then(|png| decode_png(&png));
            done(image);
        }
        Ok(())
    }));
    // SAFETY: `stream` and `handler` are live COM objects, which the call
    // references until the handler has run.
    let started = unsafe { webview.webview().CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler) };
    if let Err(e) = started
        && let Some(done) = slot.take()
    {
        done(Err(format!("WebView2 refused to capture the page: {e}")));
    }
}

/// Every byte of a memory stream, from its start.
#[cfg(windows)]
fn read_stream(stream: &windows::Win32::System::Com::IStream) -> Result<Vec<u8>, String> {
    use windows::Win32::System::Com::{STREAM_SEEK_END, STREAM_SEEK_SET};
    let mut size = 0u64;
    // SAFETY: `size` outlives the call that writes it.
    unsafe { stream.Seek(0, STREAM_SEEK_END, Some(&mut size)) }.map_err(|e| format!("the capture stream's size cannot be read: {e}"))?;
    // SAFETY: seeking a live stream to its start.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(|e| format!("the capture stream cannot be rewound: {e}"))?;
    let len = usize::try_from(size).map_err(|e| format!("a capture of {size} bytes does not fit this process: {e}"))?;
    let mut bytes = vec![0u8; len];
    let mut at = 0usize;
    while at < len {
        let want = (len - at).min(u32::MAX as usize) as u32;
        let mut got = 0u32;
        // SAFETY: the destination has room for `want` bytes past `at`,
        // and `got` outlives the call.
        unsafe { stream.Read(bytes[at..].as_mut_ptr().cast(), want, Some(&mut got)) }.ok().map_err(|e| format!("the capture stream cannot be read: {e}"))?;
        if got == 0 {
            break;
        }
        at += got as usize;
    }
    if at != len {
        return Err(format!("the capture stream gave {at} of its {len} bytes"));
    }
    Ok(bytes)
}

/// Starts WebKitGTK's snapshot of the visible part of `webview`; `done`
/// runs from the snapshot's callback on GTK's main context.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn start(webview: &WebView, done: Done) {
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
    use wry::WebViewExtUnix;
    webview.webview().snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&webkit2gtk::gio::Cancellable>, move |result| {
        done(result.map_err(|e| format!("WebKitGTK could not capture the page: {e}")).and_then(surface_image));
    });
}

/// A cairo image surface's pixels as straight RGBA. Cairo stores each
/// pixel as a native-endian 32-bit word, alpha in the top byte, with color
/// premultiplied by alpha in `ARgb32` and alpha unused in `Rgb24`.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn surface_image(surface: cairo::Surface) -> Result<Image, String> {
    let mut surface = cairo::ImageSurface::try_from(surface).map_err(|other| format!("WebKitGTK's capture is a {:?} surface, not an image", other.type_()))?;
    let premultiplied = match surface.format() {
        cairo::Format::ARgb32 => true,
        cairo::Format::Rgb24 => false,
        other => return Err(format!("WebKitGTK's capture is in cairo's {other:?} format, which this renderer does not read")),
    };
    let width = u32::try_from(surface.width()).map_err(|e| format!("a capture {} pixels wide: {e}", surface.width()))?;
    let height = u32::try_from(surface.height()).map_err(|e| format!("a capture {} pixels high: {e}", surface.height()))?;
    let stride = usize::try_from(surface.stride()).map_err(|e| format!("a capture with a stride of {}: {e}", surface.stride()))?;
    let data = surface.data().map_err(|e| format!("WebKitGTK's capture cannot be read: {e}"))?;
    let row = width as usize * 4;
    let mut rgba = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let line = data.get(y * stride..y * stride + row).ok_or_else(|| format!("a {width}x{height} capture with stride {stride} has {} bytes", data.len()))?;
        for &px in line.as_chunks::<4>().0 {
            let word = u32::from_ne_bytes(px);
            let a = if premultiplied { (word >> 24) as u8 } else { 255 };
            let straight = |c: u32| -> u8 {
                let c = c & 0xff;
                if a == 0 || a == 255 { c as u8 } else { ((c * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 }
            };
            rgba.extend_from_slice(&[straight(word >> 16), straight(word >> 8), straight(word), a]);
        }
    }
    Ok(Image { width, height, rgba })
}

/// Starts WebKit's snapshot of `webview`; `done` runs from its completion
/// handler on the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn start(webview: &WebView, done: Done) {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSError;
    use objc2_web_kit::WKSnapshotConfiguration;
    use std::cell::Cell;
    use wry::WebViewExtMacOS;

    let Some(mtm) = MainThreadMarker::new() else {
        return done(Err(String::from("a capture on macOS has to start on the main thread")));
    };
    // SAFETY: made on the main thread, as WebKit's classes require.
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    let slot = Cell::new(Some(done));
    let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let Some(done) = slot.take() else {
            return;
        };
        // SAFETY: WebKit passes an image or an error, each live or nil, for
        // the duration of the call.
        let (image, error) = unsafe { (image.as_ref(), error.as_ref()) };
        let result = match (image, error) {
            (Some(image), _) => png_of(image).and_then(|png| decode_png(&png)),
            (None, Some(error)) => Err(format!("WebKit could not capture the page: {}", error.localizedDescription())),
            (None, None) => Err(String::from("WebKit returned neither an image nor an error for the capture")),
        };
        done(result);
    });
    // SAFETY: the view, the configuration and the block are live, and
    // WebKit copies the block before returning.
    unsafe { webview.webview().takeSnapshotWithConfiguration_completionHandler(Some(&config), &block) };
}

/// An image's best bitmap as PNG bytes.
#[cfg(target_os = "macos")]
fn png_of(image: &objc2_app_kit::NSImage) -> Result<Vec<u8>, String> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::NSDictionary;
    let tiff = image.TIFFRepresentation().ok_or("WebKit's capture has no bitmap to read")?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff).ok_or("WebKit's capture cannot be read as a bitmap")?;
    // SAFETY: an empty dictionary is a valid set of PNG properties.
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.ok_or("WebKit's capture cannot be written as a PNG")?;
    Ok(png.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> Image {
        let rgba = (0..width * height).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        Image { width, height, rgba }
    }

    #[test]
    fn a_crop_scales_the_page_rectangle_to_device_pixels() {
        let shot = image(8, 8);
        let crop = crop(&shot, Rect { x: 1.0, y: 1.0, width: 2.0, height: 1.0 }, (4.0, 4.0)).expect("a crop");
        assert_eq!((crop.width, crop.height), (4, 2));
        assert_eq!(crop.rgba[0], 18, "the crop starts at pixel (2, 2)");
    }

    #[test]
    fn a_crop_outside_the_screenshot_is_refused() {
        let refused = crop(&image(4, 4), Rect { x: 10.0, y: 0.0, width: 2.0, height: 2.0 }, (4.0, 4.0));
        assert_eq!(refused, Err(String::from("the widget, 2 by 2 at (10, 0) in a page 4 by 4, lies outside the 4x4 screenshot")));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_png_decodes_to_rgba() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("a PNG header");
            writer.write_image_data(&[10, 20, 30, 40, 50, 60]).expect("PNG data");
        }
        let decoded = decode_png(&bytes).expect("a decoded PNG");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
