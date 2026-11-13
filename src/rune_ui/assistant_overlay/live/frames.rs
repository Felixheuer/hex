//! One browser's stream: connecting to it, reading its frames, and
//! keeping the latest as a thumbnail (filtered down, never below its
//! on-screen pixels) and, while shown large, in full.

use super::*;

/// Streams `preview` until it's halted, reconnecting after a failure.
pub(super) fn run(preview: &Preview, path: &std::path::Path) {
    while !preview.stop.load(Ordering::Acquire) {
        if preview.rate() == Rate::Idle {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        if let Err(why) = stream_once(preview, path) {
            // Closed on purpose for a fresh frame, or because nothing draws
            // it any more: reconnect at once, or wait while idle.
            if preview.refresh.swap(false, Ordering::AcqRel) || preview.rate() == Rate::Idle {
                continue;
            }
            set_page(preview, |page| page.failed = Some(why.clone()));
            tracing::debug!(session = %preview.session, %why, "browser preview stream ended");
            std::thread::sleep(RETRY);
        }
    }
}

pub(super) fn stream_once(preview: &Preview, path: &std::path::Path) -> Result<(), String> {
    let result = stream_connected(preview, path);
    // The stream is over: nothing writes to it any more.
    if let Ok(mut slot) = preview.writer.lock() {
        *slot = None;
    }
    result
}

pub(super) fn stream_connected(preview: &Preview, path: &std::path::Path) -> Result<(), String> {
    if preview.rate() == Rate::Idle {
        return Err("idle".into());
    }
    let port = stream::enable(path, &preview.session)?;
    let fps = preview.rate().fps().max(1);
    let (mut reader, writer) = stream::connect(port, &format!("pacing=ack&maxFps={fps}"))
        .map_err(|error| error.to_string())?;
    if let Ok(mut slot) = preview.writer.lock() {
        *slot = Some(writer.clone());
    }
    // Gone idle while it connected (its close went to no writer): drop it.
    if preview.rate() == Rate::Idle {
        writer.close();
        return Err("idle".into());
    }
    set_page(preview, |page| page.failed = None);
    let seen = AtomicU64::new(0);
    while !preview.stop.load(Ordering::Acquire) {
        let Some(text) = reader.next().map_err(|error| error.to_string())? else {
            return Err("the stream closed".into());
        };
        match stream::parse(&text) {
            Some(Message::Frame {
                seq,
                jpeg,
                viewport,
            }) => {
                if let Some((width, height, rgba)) = decode(&jpeg) {
                    let (tw, th, thumb) = fit_width(&rgba, width, height, THUMB_WIDTH);
                    preview.pixels.set(tw as u32, th as u32, thumb);
                    // The full frame only while someone looks closely.
                    if preview.rate() >= Rate::Wall {
                        preview.full.set(width as u32, height as u32, rgba);
                    }
                }
                let _ = writer.send(&stream::ack(seq));
                let first = seen.fetch_add(1, Ordering::AcqRel) == 0;
                set_page(preview, |page| {
                    page.viewport = viewport;
                    page.frames += 1;
                });
                if first {
                    changed();
                }
            }
            Some(Message::Url(url)) => {
                if set_page(preview, |page| page.url = url) {
                    changed();
                }
            }
            Some(Message::Tabs { title, url }) => {
                if set_page(preview, |page| {
                    page.title = title;
                    if !url.is_empty() {
                        page.url = url;
                    }
                }) {
                    changed();
                }
            }
            Some(Message::Other) | None => {}
        }
    }
    Ok(())
}

/// Changes `preview`'s page; whether its words changed.
pub(super) fn set_page(preview: &Preview, change: impl FnOnce(&mut Page)) -> bool {
    let Ok(mut page) = preview.page.lock() else {
        return false;
    };
    let before = (page.title.clone(), page.url.clone(), page.failed.clone());
    change(&mut page);
    before != (page.title.clone(), page.url.clone(), page.failed.clone())
}

/// A JPEG frame as RGBA at its own size.
pub(super) fn decode(jpeg: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(jpeg), options);
    let rgba = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    (width > 0 && height > 0 && rgba.len() == width * height * 4).then_some((width, height, rgba))
}

/// `rgba` scaled to `max_width` wide (kept when it's no wider), each output
/// pixel the average of the source area it covers: a box filter at the
/// exact size, so text stays legible instead of aliasing (the GPU samples
/// a streamed frame without mipmaps).
pub(in super::super) fn fit_width(
    rgba: &[u8],
    width: usize,
    height: usize,
    max_width: usize,
) -> (usize, usize, Vec<u8>) {
    if width <= max_width || max_width == 0 {
        return (width, height, rgba.to_vec());
    }
    let w = max_width;
    let h = ((height * w) as f64 / width as f64).round().max(1.0) as usize;
    let span = |out: usize, size: usize, from: usize| {
        let start = out * from / size;
        let end = ((out + 1) * from / size).max(start + 1).min(from);
        start..end
    };
    let mut result = vec![0u8; w * h * 4];
    for y in 0..h {
        let ys = span(y, h, height);
        for x in 0..w {
            let xs = span(x, w, width);
            let mut sum = [0u32; 4];
            for sy in ys.clone() {
                let row = sy * width;
                for sx in xs.clone() {
                    let at = (row + sx) * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[at + channel]);
                    }
                }
            }
            let count = (ys.len() * xs.len()) as u32;
            let at = (y * w + x) * 4;
            for (channel, total) in sum.iter().enumerate() {
                result[at + channel] = ((total + count / 2) / count) as u8;
            }
        }
    }
    (w, h, result)
}
