//! The Wayland desktop as Hex reads it: the outputs (xdg-output's logical
//! layout) and a screenshot through the desktop's Screenshot portal, which
//! asks the person once whether Hex may take screenshots and remembers the
//! answer. Wayland tells an app nothing of other windows' places, so the
//! portal's picture can't have Hex's or blocked apps' windows left out:
//! while a blocked app runs, Hex takes none.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use color_eyre::{Result, eyre::eyre};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::{
    self, ZxdgOutputManagerV1,
};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use zbus::{OwnedVariant, Variant};

use crate::assistant::computer::displays::Display;

const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";

/// A screenshot went through the portal in this run: it may take them.
static ALLOWED: AtomicBool = AtomicBool::new(false);

/// A screenshot of the whole desktop through the portal, as RGBA and its
/// size; the portal may ask the person first.
pub fn screenshot() -> Result<(Vec<u8>, u32, u32)> {
    let stop = AtomicBool::new(false);
    let mut bus = crate::linux::portal::PortalBus::open(SCREENSHOT, &stop)?
        .ok_or_else(|| eyre!("This desktop has no screenshot portal"))?;
    let results = bus.request(
        "Screenshot",
        |token| {
            (
                "",
                HashMap::from([
                    ("handle_token", Variant::from(token.to_owned())),
                    ("interactive", Variant::from(false)),
                ]),
            )
        },
        &stop,
    )?;
    let uri = results
        .get("uri")
        .and_then(|uri| <&str>::try_from(uri).ok())
        .ok_or_else(|| eyre!("The screenshot portal returned no picture"))?;
    let path = url::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| eyre!("The screenshot portal returned no file"))?;
    let bytes = std::fs::read(&path);
    // The portal leaves its picture in the person's folders: never kept.
    let _ = std::fs::remove_file(&path);
    let decoded = decode_png(&bytes?)?;
    ALLOWED.store(true, Ordering::Release);
    Ok(decoded)
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| eyre!("The screenshot isn't a picture Hex reads: {error}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| eyre!("The screenshot isn't a picture Hex reads: {error}"))?;
    let pixels = buffer[..info.buffer_size()].to_vec();
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[red, green, blue]| [*red, *green, *blue, 255])
            .collect(),
        other => {
            return Err(eyre!(
                "The screenshot's colours ({other:?}) aren't supported"
            ));
        }
    };
    Ok((rgba, info.width, info.height))
}

/// Whether the portal lets Hex take screenshots: one went through in this
/// run, or the desktop's permission store says yes for Hex.
pub fn allowed() -> bool {
    ALLOWED.load(Ordering::Acquire) || stored_permission() == Some(true)
}

/// What the permission store keeps for Hex's screenshots (GNOME's portal
/// stores the answer there; others may keep none).
fn stored_permission() -> Option<bool> {
    crate::linux::atspi::within(std::time::Duration::from_secs(1), async {
        let session = zbus::Connection::session().await.ok()?;
        let reply = session
            .call_method(
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "/org/freedesktop/impl/portal/PermissionStore",
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "Lookup",
                &("screenshot", "screenshot"),
            )
            .await
            .ok()?;
        let (permissions, _): (HashMap<String, Vec<String>>, OwnedVariant) =
            reply.body().deserialize().ok()?;
        super::super::stored_answer(&permissions, crate::linux::portal::APP_ID)
    })
    .flatten()
}

/// Asks the portal for a screenshot now (it shows its question the first
/// time) and drops the picture.
pub fn ask() {
    std::thread::spawn(|| {
        if let Err(error) = screenshot() {
            tracing::warn!(error = %format!("{error:#}"), "the screenshot portal said no");
        }
    });
}

/// The outputs in the desktop's logical layout, the one at the origin
/// first, named by their connectors ("DP-1").
pub fn displays() -> Vec<Display> {
    let Some(mut outputs) = outputs() else {
        return Vec::new();
    };
    outputs.sort_by_key(|output| (output.position != (0, 0), output.global));
    outputs
        .into_iter()
        .enumerate()
        .filter_map(|(index, output)| {
            let (width, height) = output.size?;
            let frame = (
                f64::from(output.position.0),
                f64::from(output.position.1),
                f64::from(width),
                f64::from(height),
            );
            Some(Display {
                id: (index + 1).to_string(),
                name: output
                    .name
                    .unwrap_or_else(|| format!("Display {}", index + 1)),
                frame,
                visible_frame: frame,
                scale: f64::from(output.scale.max(1)),
                is_primary: output.position == (0, 0),
                has_focus: false,
            })
        })
        .collect()
}

#[derive(Default)]
struct Output {
    global: u32,
    name: Option<String>,
    position: (i32, i32),
    size: Option<(i32, i32)>,
    scale: i32,
}

#[derive(Default)]
struct Outputs(Vec<Output>);

impl Outputs {
    fn get(&mut self, global: u32) -> Option<&mut Output> {
        self.0.iter_mut().find(|output| output.global == global)
    }
}

fn outputs() -> Option<Vec<Output>> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&connection).ok()?;
    let qh = queue.handle();
    let manager: ZxdgOutputManagerV1 = globals.bind(&qh, 2..=3, ()).ok()?;
    let found: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version.min(4)))
            .collect()
    });
    let mut outputs = Outputs::default();
    let mut bound = Vec::new();
    for (global, version) in found {
        let output = globals
            .registry()
            .bind::<WlOutput, _, _>(global, version, &qh, global);
        let xdg = manager.get_xdg_output(&output, &qh, global);
        outputs.0.push(Output {
            global,
            scale: 1,
            ..Output::default()
        });
        bound.push((output, xdg));
    }
    queue.roundtrip(&mut outputs).ok()?;
    for (output, xdg) in &bound {
        xdg.destroy();
        if output.version() >= 3 {
            output.release();
        }
    }
    manager.destroy();
    let _ = queue.roundtrip(&mut outputs);
    Some(outputs.0)
}

impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for Outputs {
    fn event(
        _: &mut Self,
        _: &ZxdgOutputManagerV1,
        _: zxdg_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, u32> for Outputs {
    fn event(
        outputs: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = outputs.get(*global) else {
            return;
        };
        match event {
            wl_output::Event::Scale { factor } => output.scale = factor,
            wl_output::Event::Name { name } => output.name = Some(name),
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for Outputs {
    fn event(
        outputs: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = outputs.get(*global) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.position = (x, y),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.size = Some((width, height));
            }
            zxdg_output_v1::Event::Name { name } if output.name.is_none() => {
                output.name = Some(name);
            }
            _ => {}
        }
    }
}
