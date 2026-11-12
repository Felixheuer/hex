//! What a question's screenshot never shows: the windows of apps the person
//! blocked (left out of the capture by ScreenCaptureKit), and password
//! fields (blacked out on the image, found through Accessibility).

/// `[x, y, width, height]` in global points.
pub type Rect = [f64; 4];

/// How long finding password fields may take, for all apps together.
const SECURE_BUDGET: std::time::Duration = std::time::Duration::from_millis(250);

/// An app name or bundle id as compared: case, soft hyphens, and spacing
/// don't count.
fn key(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '\u{ad}')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The processes among `owners` (pid, app name) that are blocked apps, by
/// name or by bundle id (`bundle_of`).
pub fn blocked_pids(
    owners: &[(i32, String)],
    bundle_of: impl Fn(i32) -> Option<String>,
    blocked: &[String],
) -> Vec<i32> {
    let blocked: Vec<String> = blocked.iter().map(|app| key(app)).collect();
    let mut pids: Vec<i32> = owners
        .iter()
        .filter(|(pid, name)| {
            blocked.contains(&key(name))
                || bundle_of(*pid).is_some_and(|bundle| blocked.contains(&key(&bundle)))
        })
        .map(|(pid, _)| *pid)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Paints `rects` (global points, padded a little) black on an RGBA image of
/// `display` (global points) that is `size` pixels.
pub fn black_out(rgba: &mut [u8], (width, height): (u32, u32), display: Rect, rects: &[Rect]) {
    if display[2] <= 0.0 || display[3] <= 0.0 {
        return;
    }
    let scale_x = f64::from(width) / display[2];
    let scale_y = f64::from(height) / display[3];
    for rect in rects {
        let pad = 2.0;
        let left = ((rect[0] - display[0] - pad) * scale_x).floor().max(0.0);
        let top = ((rect[1] - display[1] - pad) * scale_y).floor().max(0.0);
        let right = ((rect[0] + rect[2] - display[0] + pad) * scale_x)
            .ceil()
            .min(f64::from(width));
        let bottom = ((rect[1] + rect[3] - display[1] + pad) * scale_y)
            .ceil()
            .min(f64::from(height));
        if right <= left || bottom <= top {
            continue;
        }
        for y in top as usize..bottom as usize {
            for x in left as usize..right as usize {
                let at = (y * width as usize + x) * 4;
                if let Some(pixel) = rgba.get_mut(at..at + 4) {
                    pixel.copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
    }
}

/// The blocked apps (`assistant.blocked_apps`, its defaults until settings
/// are applied).
pub(super) fn blocked_apps() -> Vec<String> {
    crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.blocked_apps)
        .unwrap_or_else(|| crate::app_settings::AssistantSettings::default().blocked_apps)
}

/// The processes of blocked apps that show windows now.
#[cfg(target_os = "macos")]
pub fn blocked_pids_now() -> Vec<i32> {
    let blocked = blocked_apps();
    if blocked.is_empty() {
        return Vec::new();
    }
    let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
    let owners: Vec<(i32, String)> = windows
        .iter()
        .map(|window| (window.pid, window.owner.clone()))
        .collect();
    blocked_pids(&owners, bundle_id, &blocked)
}

#[cfg(target_os = "macos")]
fn bundle_id(pid: i32) -> Option<String> {
    objc2::rc::autoreleasepool(|_| {
        objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?
            .bundleIdentifier()
            .map(|id| id.to_string())
    })
}

/// The password fields of apps with windows on `display` (global points),
/// within a quarter second for all of them.
#[cfg(target_os = "macos")]
pub fn secure_fields_on(display: Rect) -> Vec<Rect> {
    let deadline = std::time::Instant::now() + SECURE_BUDGET;
    let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
    let overlaps = |bounds: [f64; 4]| {
        bounds[0] < display[0] + display[2]
            && bounds[0] + bounds[2] > display[0]
            && bounds[1] < display[1] + display[3]
            && bounds[1] + bounds[3] > display[1]
    };
    let mut pids: Vec<i32> = Vec::new();
    for window in &windows {
        if window.layer == 0
            && window.alpha > 0.0
            && overlaps(window.bounds)
            && !pids.contains(&window.pid)
        {
            pids.push(window.pid);
        }
    }
    let mut rects = Vec::new();
    for pid in pids {
        if std::time::Instant::now() >= deadline {
            tracing::debug!("password fields: out of time; some apps unchecked");
            break;
        }
        rects.extend(crate::accessibility::secure_field_frames(pid, deadline));
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_apps_are_found_by_name_or_bundle_id() {
        let owners = [
            (10, "1Password 7".to_owned()),
            (11, "Mail".to_owned()),
            (12, "Keychain\u{ad} Access".to_owned()),
            (10, "1Password 7".to_owned()),
        ];
        let bundle = |pid: i32| (pid == 10).then(|| "com.agilebits.onepassword7".to_owned());
        let blocked = [
            "Keychain Access".to_owned(),
            "com.agilebits.onepassword7".to_owned(),
        ];
        assert_eq!(blocked_pids(&owners, bundle, &blocked), [10, 12]);
        assert!(blocked_pids(&owners, bundle, &[]).is_empty());
    }

    #[test]
    fn password_fields_are_blacked_out_where_the_image_shows_them() {
        // This Mac's M27Q (-604,-1440, 2560×1440 points) captured at 1280×720.
        let display = [-604.0, -1440.0, 2560.0, 1440.0];
        let (width, height) = (1280_u32, 720_u32);
        let mut rgba = vec![255_u8; (width * height * 4) as usize];
        // A field 200×20 points at (-404,-1240): pixels (100,100)-(200,110).
        black_out(
            &mut rgba,
            (width, height),
            display,
            &[[-404.0, -1240.0, 200.0, 20.0]],
        );
        let pixel = |x: u32, y: u32| {
            let at = ((y * width + x) * 4) as usize;
            [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
        };
        assert_eq!(pixel(150, 105), [0, 0, 0, 255]);
        assert_eq!(pixel(100, 100), [0, 0, 0, 255]);
        assert_eq!(pixel(150, 120), [255, 255, 255, 255]);
        assert_eq!(pixel(50, 50), [255, 255, 255, 255]);
        // A field on another display, or off the image, changes nothing.
        let before = rgba.clone();
        black_out(
            &mut rgba,
            (width, height),
            display,
            &[[100.0, 100.0, 200.0, 20.0]],
        );
        black_out(
            &mut rgba,
            (width, height),
            display,
            &[[2000.0, -20.0, 400.0, 100.0]],
        );
        assert_eq!(rgba, before);
    }

    /// Live, read-only: looking for password fields on the main display
    /// stays within its budget.
    #[cfg(target_os = "macos")]
    #[test]
    fn finding_password_fields_stays_within_its_budget() {
        let started = std::time::Instant::now();
        let fields = secure_fields_on([0.0, 0.0, 1800.0, 1169.0]);
        let took = started.elapsed();
        let blocked = blocked_pids_now();
        eprintln!(
            "{} password fields in {took:?}; blocked app processes: {blocked:?}",
            fields.len()
        );
        assert!(
            took < SECURE_BUDGET + std::time::Duration::from_millis(300),
            "{took:?}"
        );
    }

    #[test]
    fn a_field_on_a_1x_display_maps_to_its_own_pixels() {
        // A 1× display right of the main one, captured at its full 1920×1080.
        let display = [1800.0, 0.0, 1920.0, 1080.0];
        let mut rgba = vec![255_u8; 1920 * 1080 * 4];
        black_out(
            &mut rgba,
            (1920, 1080),
            display,
            &[[1900.0, 50.0, 10.0, 10.0]],
        );
        let at = (55 * 1920 + 105) * 4;
        assert_eq!(&rgba[at..at + 4], &[0, 0, 0, 255]);
        let outside = (55 * 1920 + 120) * 4;
        assert_eq!(&rgba[outside..outside + 4], &[255, 255, 255, 255]);
    }
}
