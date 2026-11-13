//! A window's text without Accessibility (`hex_read_window`): its picture
//! (ScreenCaptureKit, wherever it is, parked or covered) read by the
//! Vision framework's `VNRecognizeTextRequest`, as lines with where they
//! sit. For apps that show no Accessibility tree (Citrix, remote desktops,
//! games, canvases). The last reading per window is kept while its picture
//! hasn't changed, so asking twice costs one capture.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

/// One line of text in a window.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
    pub text: String,
    /// Where it sits in the window, in points from its top-left corner.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// 0 to 1.
    pub confidence: f32,
}

/// A window's text, read.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Reading {
    pub lines: Vec<Line>,
    /// Read again (`false`) or from the last picture's reading.
    pub cached: bool,
}

/// The long side the window is captured at: enough for small print.
const EDGE: u32 = 2400;

/// Reads window `number` (its size in points: `size`, to place the lines).
pub fn read(number: u32, size: Option<(f64, f64)>) -> Result<Reading, String> {
    let (width, height, rgba) = capture(number)?;
    let key = fingerprint(&rgba);
    static CACHE: Mutex<Option<HashMap<u32, (u64, Vec<Line>)>>> = Mutex::new(None);
    if let Some((seen, lines)) = CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(|cache| cache.get(&number))
        && *seen == key
    {
        return Ok(Reading {
            lines: lines.clone(),
            cached: true,
        });
    }
    let boxes = recognize(width, height, &rgba)?;
    let (points_w, points_h) = size.unwrap_or((f64::from(width), f64::from(height)));
    let lines = place(&boxes, points_w, points_h);
    CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(number, (key, lines.clone()));
    Ok(Reading {
        lines,
        cached: false,
    })
}

/// Text as Vision gives it: the box normalized (0 to 1) with its origin at
/// the bottom left.
#[derive(Clone, Debug, PartialEq)]
pub struct Recognized {
    pub text: String,
    pub confidence: f32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `boxes` in a window of `width`×`height` points, top to bottom, then left
/// to right.
pub fn place(boxes: &[Recognized], width: f64, height: f64) -> Vec<Line> {
    let mut lines: Vec<Line> = boxes
        .iter()
        .filter(|found| !found.text.trim().is_empty())
        .map(|found| Line {
            text: found.text.trim().to_owned(),
            x: (found.x * width).round(),
            y: ((1.0 - found.y - found.height) * height).round(),
            width: (found.width * width).round(),
            height: (found.height * height).round(),
            confidence: found.confidence,
        })
        .collect();
    lines.sort_by(|a, b| a.y.total_cmp(&b.y));
    // Rows: lines whose tops are within half a line of the row's first,
    // each read left to right.
    let mut rows: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        match rows.last_mut() {
            Some(row) if (line.y - row[0].y).abs() < row[0].height.min(line.height) / 2.0 => {
                row.push(line);
            }
            _ => rows.push(vec![line]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| a.x.total_cmp(&b.x));
            row
        })
        .collect()
}

/// The lines as plain text, a row per line.
pub fn text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A cheap fingerprint of a picture (every 97th byte and the length).
fn fingerprint(rgba: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ rgba.len() as u64;
    for byte in rgba.iter().step_by(97) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn capture(number: u32) -> Result<(u32, u32, Vec<u8>), String> {
    #[cfg(target_os = "macos")]
    {
        scrawl_capture::capture::capture_window(number, EDGE).ok_or_else(|| {
            "Hex couldn't take the window's picture (it closed, or Screen Recording isn't \
             allowed)."
                .to_owned()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (number, EDGE);
        Err("Reading a window's text from its picture works on macOS only.".into())
    }
}

fn recognize(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<Recognized>, String> {
    #[cfg(target_os = "macos")]
    {
        objc2::rc::autoreleasepool(|_| vision::recognize(width, height, rgba))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (width, height, rgba);
        Err("Reading a window's text from its picture works on macOS only.".into())
    }
}

#[cfg(target_os = "macos")]
mod vision {
    use std::ffi::c_void;

    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::{NSArray, NSRect, NSString};

    use super::Recognized;

    #[link(name = "Vision", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGColorSpaceRelease(space: *mut c_void);
        fn CGDataProviderCreateWithData(
            info: *mut c_void,
            data: *const c_void,
            size: usize,
            release: Option<unsafe extern "C" fn(*mut c_void, *const c_void, usize)>,
        ) -> *mut c_void;
        fn CGDataProviderRelease(provider: *mut c_void);
        #[allow(clippy::too_many_arguments)]
        fn CGImageCreate(
            width: usize,
            height: usize,
            bits_per_component: usize,
            bits_per_pixel: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
            provider: *mut c_void,
            decode: *const f64,
            should_interpolate: bool,
            intent: i32,
        ) -> *mut c_void;
        fn CGImageRelease(image: *mut c_void);
    }

    /// `kCGImageAlphaPremultipliedLast`.
    const RGBA: u32 = 1;
    /// `VNRequestTextRecognitionLevelAccurate`.
    const ACCURATE: isize = 0;

    /// Releases an object made with alloc/init.
    struct Owned(*mut AnyObject);

    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a +1 object this owns.
                let _: () = unsafe { msg_send![self.0, release] };
            }
        }
    }

    pub fn recognize(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<Recognized>, String> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || rgba.len() < w * h * 4 {
            return Err("the window's picture is empty".into());
        }
        let handler_class =
            AnyClass::get(c"VNImageRequestHandler").ok_or("the Vision framework isn't here")?;
        let request_class =
            AnyClass::get(c"VNRecognizeTextRequest").ok_or("the Vision framework isn't here")?;
        // SAFETY: CoreGraphics calls with checked sizes; `rgba` outlives the
        // image (released before returning), and the provider doesn't copy
        // or free it.
        let image = unsafe {
            let space = CGColorSpaceCreateDeviceRGB();
            let provider = CGDataProviderCreateWithData(
                std::ptr::null_mut(),
                rgba.as_ptr().cast(),
                w * h * 4,
                None,
            );
            let image = CGImageCreate(
                w,
                h,
                8,
                32,
                w * 4,
                space,
                RGBA,
                provider,
                std::ptr::null(),
                false,
                0,
            );
            CGDataProviderRelease(provider);
            CGColorSpaceRelease(space);
            image
        };
        if image.is_null() {
            return Err("couldn't make an image of the window's picture".into());
        }
        let result = (|| {
            // SAFETY: Vision's documented initializers and properties, with
            // the types they take; every alloc/init is released by `Owned`.
            unsafe {
                let handler: *mut AnyObject = msg_send![handler_class, alloc];
                let options: *mut AnyObject = msg_send![
                    AnyClass::get(c"NSDictionary").ok_or("no NSDictionary")?,
                    dictionary
                ];
                let handler = Owned(msg_send![handler, initWithCGImage: image, options: options]);
                if handler.0.is_null() {
                    return Err("Vision couldn't take the picture".to_owned());
                }
                let request: *mut AnyObject = msg_send![request_class, alloc];
                let request = Owned(msg_send![request, init]);
                if request.0.is_null() {
                    return Err("Vision couldn't make a text request".to_owned());
                }
                let _: () = msg_send![request.0, setRecognitionLevel: ACCURATE];
                let _: () = msg_send![request.0, setUsesLanguageCorrection: Bool::YES];
                let languages = NSArray::from_retained_slice(&[
                    NSString::from_str("de-DE"),
                    NSString::from_str("en-US"),
                ]);
                let _: () = msg_send![request.0, setRecognitionLanguages: &*languages];
                let requests: *mut AnyObject = msg_send![AnyClass::get(c"NSArray").ok_or("no NSArray")?, arrayWithObject: request.0];
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let done: Bool = msg_send![handler.0, performRequests: requests, error: &mut error];
                if !done.as_bool() {
                    return Err("Vision couldn't read the window's text".to_owned());
                }
                let results: *mut AnyObject = msg_send![request.0, results];
                if results.is_null() {
                    return Ok(Vec::new());
                }
                let count: usize = msg_send![results, count];
                let mut found = Vec::with_capacity(count);
                for index in 0..count {
                    let observation: *mut AnyObject = msg_send![results, objectAtIndex: index];
                    let candidates: *mut AnyObject = msg_send![observation, topCandidates: 1_usize];
                    let first: *mut AnyObject = msg_send![candidates, firstObject];
                    if first.is_null() {
                        continue;
                    }
                    let string: *mut NSString = msg_send![first, string];
                    let confidence: f32 = msg_send![first, confidence];
                    let frame: NSRect = msg_send![observation, boundingBox];
                    let text = string
                        .as_ref()
                        .map(|text| text.to_string())
                        .unwrap_or_default();
                    found.push(Recognized {
                        text,
                        confidence,
                        x: frame.origin.x,
                        y: frame.origin.y,
                        width: frame.size.width,
                        height: frame.size.height,
                    });
                }
                Ok(found)
            }
        })();
        // SAFETY: made above, released once.
        unsafe { CGImageRelease(image) };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_placed_top_down_in_window_points() {
        let boxes = [
            Recognized {
                text: "unten".into(),
                confidence: 0.9,
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.05,
            },
            Recognized {
                text: "rechts".into(),
                confidence: 0.8,
                x: 0.6,
                y: 0.8,
                width: 0.2,
                height: 0.05,
            },
            Recognized {
                text: "links".into(),
                confidence: 1.0,
                x: 0.1,
                y: 0.81,
                width: 0.2,
                height: 0.05,
            },
            Recognized {
                text: "  ".into(),
                confidence: 1.0,
                x: 0.0,
                y: 0.0,
                width: 0.1,
                height: 0.1,
            },
        ];
        let lines = place(&boxes, 1000.0, 800.0);
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["links", "rechts", "unten"]);
        assert_eq!(
            (lines[0].x, lines[0].y, lines[0].width),
            (100.0, 112.0, 200.0)
        );
        assert_eq!(text(&lines), "links\nrechts\nunten");
    }

    #[test]
    fn the_same_picture_has_the_same_fingerprint() {
        let a = vec![7_u8; 4000];
        let mut b = a.clone();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        b[97 * 3] = 8;
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }
}
