//! The monitor the window is on, as the compositor describes it through the Wayland colour
//! management protocol (`wp_color_manager_v1`: KDE, GNOME and others). X11, or a compositor
//! without the protocol, reports SDR.
use super::DisplayInfo;
use anyhow::{Context, Result};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output::WlOutput, wl_registry::WlRegistry},
};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_output_v1::{self as output_v1, WpColorManagementOutputV1},
    wp_color_manager_v1::{self as manager_v1, WpColorManagerV1},
    wp_image_description_info_v1::{self as info_v1, WpImageDescriptionInfoV1},
    wp_image_description_v1::{self as description_v1, WpImageDescriptionV1},
};

/// What the events of one query collect.
#[derive(Default)]
struct State {
    /// `wl_output` names, by bind order (`wl_output` v4).
    names: Vec<String>,
    /// The description's information has arrived, or it failed.
    done: bool,
    /// `image_description_changed`: HDR toggled, SDR brightness moved.
    changed: bool,
    wide: bool,
    pq: bool,
    reference: f32,
    max: f32,
    target_min: f32,
    target_max: f32,
    max_fall: f32,
}

/// Holds the connection the description came from, to hear when it stops being current.
pub struct Monitor {
    pub info: DisplayInfo,
    watch: Option<(EventQueue<State>, State, WpColorManagementOutputV1)>,
}
impl Monitor {
    /// `output` is the window's output name (winit's `MonitorHandle::name`, the same as
    /// `wl_output.name`); `None` for a window on no output, as a hidden one, which stays SDR so
    /// headless runs don't depend on the desktop.
    pub unsafe fn query(output: Option<&str>) -> Result<Self> {
        let info = DisplayInfo {
            name: output.unwrap_or_default().into(),
            ..Default::default()
        };
        let watched = output.and_then(|name| describe(name).ok());
        Ok(match watched {
            Some((queue, state, watch)) => Self {
                info: DisplayInfo {
                    // A display with a wider target than sRGB is in the protocol's PQ or
                    // extended space even at no headroom: an HDR surface reaches all of it.
                    hdr: state.pq || state.wide || state.target_max > state.reference * 1.01,
                    peak_nits: if state.target_max > 0. {
                        state.target_max
                    } else {
                        state.max
                    },
                    full_frame_nits: state.max_fall,
                    min_nits: state.target_min,
                    sdr_white_nits: state.reference,
                    ..info
                },
                watch: Some((queue, state, watch)),
            },
            None => Self { info, watch: None },
        })
    }
    /// False once the compositor said the description changed.
    pub unsafe fn current(&mut self) -> bool {
        let Some((queue, state, _)) = &mut self.watch else {
            return true;
        };
        // Doesn't wait: reads what the socket holds now.
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = queue.dispatch_pending(state);
        !state.changed
    }
}

/// Binds the colour manager, finds `name` among the outputs and reads its image description.
fn describe(name: &str) -> Result<(EventQueue<State>, State, WpColorManagementOutputV1)> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let manager: WpColorManagerV1 = globals.bind(&qh, 1..=1, ())?;
    let outputs: Vec<WlOutput> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|g| g.interface == "wl_output" && g.version >= 4)
            .enumerate()
            .map(|(i, g)| globals.registry().bind(g.name, 4, &qh, i))
            .collect()
    });
    let mut state = State {
        names: vec![String::new(); outputs.len()],
        ..Default::default()
    };
    queue.roundtrip(&mut state)?;
    let index = state
        .names
        .iter()
        .position(|n| n == name)
        .with_context(|| format!("no Wayland output {name}"))?;
    let watch = manager.get_output(&outputs[index], &qh, ());
    let description = watch.get_image_description(&qh, ());
    // This runs on the UI thread: a compositor that never answers must not hang the app.
    let deadline = Instant::now() + Duration::from_secs(1);
    while !state.done {
        // Dispatching sends requests (Ready asks for the information): flush after it.
        queue.dispatch_pending(&mut state)?;
        queue.flush()?;
        if state.done {
            break;
        }
        if let Some(guard) = queue.prepare_read() {
            let left = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as i32;
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, left) };
            anyhow::ensure!(ready > 0, "no image description for {name} in time");
            guard.read()?;
        }
    }
    description.destroy();
    anyhow::ensure!(state.reference > 0., "no image description for {name}");
    Ok((queue, state, watch))
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: <WlOutput as wayland_client::Proxy>::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_output::Event::Name { name } = event {
            state.names[*index] = name;
        }
    }
}
impl Dispatch<WpColorManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpColorManagerV1,
        _: manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WpColorManagementOutputV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpColorManagementOutputV1,
        event: output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let output_v1::Event::ImageDescriptionChanged = event {
            state.changed = true;
        }
    }
}
impl Dispatch<WpImageDescriptionV1, ()> for State {
    fn event(
        state: &mut Self,
        description: &WpImageDescriptionV1,
        event: description_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            description_v1::Event::Ready { .. } => {
                description.get_information(qh, ());
            }
            description_v1::Event::Failed { .. } => state.done = true,
            _ => {}
        }
    }
}
impl Dispatch<WpImageDescriptionInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpImageDescriptionInfoV1,
        event: info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use manager_v1::{Primaries, TransferFunction};
        match event {
            info_v1::Event::PrimariesNamed {
                primaries: WEnum::Value(p),
            } => state.wide = p != Primaries::Srgb,
            info_v1::Event::TfNamed {
                tf: WEnum::Value(tf),
            } => state.pq = tf == TransferFunction::St2084Pq,
            info_v1::Event::Luminances {
                max_lum,
                reference_lum,
                ..
            } => {
                state.max = max_lum as f32;
                state.reference = reference_lum as f32;
            }
            // Minimums are in units of 0.0001 cd/m².
            info_v1::Event::TargetLuminance { min_lum, max_lum } => {
                state.target_min = min_lum as f32 / 10000.;
                state.target_max = max_lum as f32;
            }
            info_v1::Event::TargetMaxFall { max_fall } => state.max_fall = max_fall as f32,
            info_v1::Event::Done => state.done = true,
            _ => {}
        }
    }
}
