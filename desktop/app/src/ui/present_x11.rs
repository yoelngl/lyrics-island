//! Linux window presentation over X11 (PutImage + Shape input mask) or native Wayland (wl_shm + WlRegion).

use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::fd::AsFd;
use std::path::Path;

use memmap2::MmapMut;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tiny_skia::Pixmap;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_compositor::WlCompositor,
    wl_region::WlRegion,
    wl_registry::{self, WlRegistry},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use winit::window::Window;
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClipOrdering, ConnectionExt as _, CreateGCAux, ImageFormat, ImageOrder,
    PropMode, Rectangle,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

type InputKey = (Option<(i32, i32, u32, u32)>, u32, bool);

pub struct Presenter {
    inner: Inner,
    /// Cached `(physical_rect, scale_bits, click_through)` so input shapes are updated only when changed.
    last_input: Option<InputKey>,
}

enum Inner {
    X11(X11Presenter),
    Wayland(WaylandPresenter),
}

impl Presenter {
    pub fn new(window: &Window) -> Self {
        let wh = window.window_handle().expect("window handle").as_raw();
        let dh = window.display_handle().expect("display handle").as_raw();
        let inner = match (wh, dh) {
            (RawWindowHandle::Xlib(h), _) => Inner::X11(X11Presenter::new(h.window as u32)),
            (RawWindowHandle::Xcb(h), _) => Inner::X11(X11Presenter::new(h.window.get())),
            (RawWindowHandle::Wayland(wh), RawDisplayHandle::Wayland(dh)) => {
                Inner::Wayland(WaylandPresenter::new(dh.display.as_ptr(), wh.surface.as_ptr()))
            }
            _ => panic!("lyrics-island needs an X11 or Wayland window"),
        };
        Self { inner, last_input: None }
    }

    /// Call right before mapping: EWMH says withdrawn windows declare their state via the property,
    /// and WMs drop it on unmap. Non-overlay windows (Settings) stay normal taskbar windows.
    pub fn prepare_show(&self, overlay: bool) {
        if let Inner::X11(x11) = &self.inner {
            x11.prepare_show(overlay);
        }
    }

    /// Restricts mouse hit-testing to `rect` (physical px) when `Some`, or the full window when `None`.
    pub fn set_input_rect(&mut self, rect: Option<(i32, i32, u32, u32)>, scale: f32, click_through: bool) {
        let key = (rect, scale.to_bits(), click_through);
        if self.last_input == Some(key) {
            return;
        }
        self.last_input = Some(key);
        match &mut self.inner {
            Inner::X11(x11) => x11.set_input_rect(rect, click_through),
            Inner::Wayland(wl) => wl.set_input_rect(rect, scale, click_through),
        }
    }

    /// Blits `damage` (`Some((x, y, w, h))` in physical px) or the full pixmap when `None`.
    pub fn present(&mut self, _window: &Window, pm: &Pixmap, damage: Option<(u32, u32, u32, u32)>) {
        match &mut self.inner {
            Inner::X11(x11) => x11.present(pm, damage),
            Inner::Wayland(wl) => wl.present(pm, damage),
        }
    }
}

struct X11Presenter {
    conn: RustConnection,
    win: u32,
    gc: u32,
    state: [u32; 5],
    /// 32 with winit's ARGB visual; 24 when the server offers none (no per-pixel alpha then).
    depth: u8,
    has_shape: bool,
    buf: Vec<u8>,
}

impl X11Presenter {
    fn new(win: u32) -> Self {
        let (conn, _) = x11rb::connect(None).expect("lyrics-island needs an X11 display");
        let gc = conn.generate_id().expect("X11 id");
        conn.create_gc(gc, win, &CreateGCAux::new()).expect("X11 create_gc");
        // winit leaves background None, so on resize/remap the server seeds the new window pixmap with a copy
        // of the screen (stale apps, old pill frames) and damage-only presents never overwrite it. Pixel 0 is
        // transparent on the ARGB visual, so the server clears instead and sends Expose for a redraw.
        let _ = conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().background_pixel(0));
        let atom = |name: &[u8]| conn.intern_atom(false, name).ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.atom);
        let state = [
            atom(b"_NET_WM_STATE"),
            atom(b"_NET_WM_STATE_SKIP_TASKBAR"),
            atom(b"_NET_WM_STATE_SKIP_PAGER"),
            atom(b"_NET_WM_STATE_ABOVE"),
            atom(b"_NET_WM_USER_TIME"),
        ];
        let depth = conn.get_geometry(win).ok().and_then(|c| c.reply().ok()).map_or(32, |g| g.depth);
        let has_shape = conn
            .extension_information(shape::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();
        Self { conn, win, gc, state, depth, has_shape, buf: Vec::new() }
    }

    fn prepare_show(&self, overlay: bool) {
        if !overlay {
            return;
        }
        let [prop, skip_taskbar, skip_pager, above, user_time] = self.state;
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.win,
            prop,
            AtomEnum::ATOM,
            &[skip_taskbar, skip_pager, above],
        );
        // User time 0 asks the WM not to steal focus when mapping the overlay.
        let _ = self.conn.change_property32(PropMode::REPLACE, self.win, user_time, AtomEnum::CARDINAL, &[0]);
        // Round trip so the properties land before winit maps the window on its own connection.
        let _ = self.conn.get_input_focus().map(|c| c.reply());
    }

    fn set_input_rect(&self, rect: Option<(i32, i32, u32, u32)>, click_through: bool) {
        if !self.has_shape {
            return;
        }
        if click_through {
            let _ = self.conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                self.win,
                0,
                0,
                &[],
            );
        } else if let Some((x, y, w, h)) = rect {
            let rects: &[Rectangle] = if w == 0 || h == 0 {
                &[]
            } else {
                &[Rectangle {
                    x: x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                    y: y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                    width: w.min(u16::MAX as u32) as u16,
                    height: h.min(u16::MAX as u32) as u16,
                }]
            };
            let _ = self.conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                self.win,
                0,
                0,
                rects,
            );
        } else {
            // Reset input region to the default full window rectangle.
            let _ = self.conn.shape_mask(shape::SO::SET, shape::SK::INPUT, self.win, 0, 0, x11rb::NONE);
        }
        let _ = self.conn.flush();
    }

    fn present(&mut self, pm: &Pixmap, damage: Option<(u32, u32, u32, u32)>) {
        let (pw, ph) = (pm.width() as usize, pm.height() as usize);
        if pw == 0 || ph == 0 {
            return;
        }
        let (dx, dy, dw, dh) = match damage {
            Some((x, y, w, h)) => {
                let x0 = (x as usize).min(pw);
                let y0 = (y as usize).min(ph);
                let w0 = (w as usize).min(pw - x0);
                let h0 = (h as usize).min(ph - y0);
                if w0 == 0 || h0 == 0 {
                    return;
                }
                (x0, y0, w0, h0)
            }
            None => (0, 0, pw, ph),
        };

        let msb = self.conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
        let src = pm.data();
        self.buf.clear();
        self.buf.reserve(dw * dh * 4);
        for row in dy..dy + dh {
            let row_slice = &src[(row * pw + dx) * 4..(row * pw + dx + dw) * 4];
            for p in row_slice.as_chunks::<4>().0 {
                let px = if msb { [p[3], p[0], p[1], p[2]] } else { [p[2], p[1], p[0], p[3]] };
                self.buf.extend_from_slice(&px);
            }
        }

        let stride = dw * 4;
        // PutImage header is 24 bytes; BIG-REQUESTS adds 4 more.
        let rows = ((self.conn.maximum_request_bytes() - 28) / stride).clamp(1, dh.max(1));
        let mut y = 0;
        while y < dh {
            let n = rows.min(dh - y);
            let _ = self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.win,
                self.gc,
                dw as u16,
                n as u16,
                dx as i16,
                (dy + y) as i16,
                0,
                self.depth,
                &self.buf[y * stride..(y + n) * stride],
            );
            y += n;
        }
        let _ = self.conn.flush();
        // Unchecked request errors are queued as events; drain so they can't pile up.
        while let Ok(Some(_)) = self.conn.poll_for_event() {}
    }
}

impl Drop for X11Presenter {
    fn drop(&mut self) {
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.flush();
    }
}

#[derive(Default)]
struct RegistryState {
    shm: Option<WlShm>,
    compositor: Option<WlCompositor>,
}

impl Dispatch<WlRegistry, ()> for RegistryState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_shm" if state.shm.is_none() => {
                    state.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_compositor" if state.compositor.is_none() => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(RegistryState: ignore WlShm);
delegate_noop!(RegistryState: ignore WlShmPool);
delegate_noop!(RegistryState: ignore WlBuffer);
delegate_noop!(RegistryState: ignore WlCompositor);
delegate_noop!(RegistryState: ignore WlRegion);

struct ShmSlot {
    file: File,
    mmap: MmapMut,
    pool: WlShmPool,
    cap: usize,
    buffer: WlBuffer,
    dims: (u32, u32),
}

struct WaylandPresenter {
    conn: Connection,
    event_queue: EventQueue<RegistryState>,
    qh: QueueHandle<RegistryState>,
    state: RegistryState,
    surface: WlSurface,
    shm: WlShm,
    compositor: WlCompositor,
    slot: Option<ShmSlot>,
}

impl WaylandPresenter {
    fn new(display: *mut c_void, surface: *mut c_void) -> Self {
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
            .expect("Wayland surface id");
        let surface = WlSurface::from_id(&conn, id).expect("Wayland surface");

        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        let _registry = conn.display().get_registry(&qh, ());
        let mut state = RegistryState::default();
        event_queue.roundtrip(&mut state).expect("Wayland registry roundtrip");
        let shm = state.shm.clone().expect("Wayland wl_shm global");
        let compositor = state.compositor.clone().expect("Wayland wl_compositor global");

        Self { conn, event_queue, qh, state, surface, shm, compositor, slot: None }
    }

    fn set_input_rect(&mut self, rect: Option<(i32, i32, u32, u32)>, scale: f32, click_through: bool) {
        let s = scale.max(1.0);
        if click_through {
            let region = self.compositor.create_region(&self.qh, ());
            self.surface.set_input_region(Some(&region));
            region.destroy();
        } else if let Some((x, y, w, h)) = rect {
            let region = self.compositor.create_region(&self.qh, ());
            if w > 0 && h > 0 {
                let lx = (x as f32 / s).floor() as i32;
                let ly = (y as f32 / s).floor() as i32;
                let lw = ((w as f32 / s).ceil() as i32).max(1);
                let lh = ((h as f32 / s).ceil() as i32).max(1);
                region.add(lx, ly, lw, lh);
            }
            self.surface.set_input_region(Some(&region));
            region.destroy();
        } else {
            self.surface.set_input_region(None);
        }
        self.surface.commit();
        let _ = self.conn.flush();
    }

    fn present(&mut self, pm: &Pixmap, damage: Option<(u32, u32, u32, u32)>) {
        let (w, h) = (pm.width(), pm.height());
        if w == 0 || h == 0 {
            return;
        }
        let byte_len = (w as usize) * (h as usize) * 4;
        let stride = (w as i32) * 4;

        let shm = &self.shm;
        let qh = &self.qh;
        let mut full = self.slot.is_none();
        let slot = self.slot.get_or_insert_with(|| {
            let file = create_shm_file();
            file.set_len(byte_len as u64).expect("Wayland shm set_len");
            let mmap = unsafe { MmapMut::map_mut(&file) }.expect("Wayland shm mmap");
            let pool = shm.create_pool(file.as_fd(), byte_len as i32, qh, ());
            let buffer = pool.create_buffer(
                0,
                w as i32,
                h as i32,
                stride,
                wl_shm::Format::Argb8888,
                qh,
                (),
            );
            ShmSlot { file, mmap, pool, cap: byte_len, buffer, dims: (w, h) }
        });
        if byte_len > slot.cap {
            let cap = byte_len.max(slot.cap.saturating_mul(2));
            slot.file.set_len(cap as u64).expect("Wayland shm resize");
            slot.mmap = unsafe { MmapMut::map_mut(&slot.file) }.expect("Wayland shm mmap");
            slot.pool.resize(cap as i32);
            slot.cap = cap;
            full = true;
        }
        let old_buffer = if slot.dims != (w, h) {
            let new_buf = slot.pool.create_buffer(
                0,
                w as i32,
                h as i32,
                stride,
                wl_shm::Format::Argb8888,
                qh,
                (),
            );
            slot.dims = (w, h);
            full = true;
            Some(std::mem::replace(&mut slot.buffer, new_buf))
        } else {
            None
        };

        let (dx, dy, dw, dh) = match damage.filter(|_| !full) {
            Some((x, y, rw, rh)) => {
                let x0 = x.min(w) as usize;
                let y0 = y.min(h) as usize;
                let w0 = (rw as usize).min(w as usize - x0);
                let h0 = (rh as usize).min(h as usize - y0);
                (x0, y0, w0, h0)
            }
            None => (0, 0, w as usize, h as usize),
        };
        if dw == 0 || dh == 0 {
            return;
        }

        let pw = w as usize;
        let src = pm.data();
        for row in dy..dy + dh {
            let s_row = &src[(row * pw + dx) * 4..(row * pw + dx + dw) * 4];
            let d_row = &mut slot.mmap[(row * pw + dx) * 4..(row * pw + dx + dw) * 4];
            for (dst, s) in d_row.as_chunks_mut::<4>().0.iter_mut().zip(s_row.as_chunks::<4>().0) {
                dst.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
        }

        self.surface.attach(Some(&slot.buffer), 0, 0);
        self.surface.damage_buffer(dx as i32, dy as i32, dw as i32, dh as i32);
        self.surface.commit();
        if let Some(old) = old_buffer {
            old.destroy();
        }
        let _ = self.conn.flush();
        let _ = self.event_queue.dispatch_pending(&mut self.state);
    }
}

impl Drop for WaylandPresenter {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.buffer.destroy();
            slot.pool.destroy();
        }
        let _ = self.conn.flush();
    }
}

fn try_shm_in(dir: &Path) -> io::Result<File> {
    let pid = std::process::id();
    for i in 0..1024u32 {
        let path = dir.join(format!("lyrics-island-shm-{pid}-{i}"));
        match OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
            Ok(file) => {
                let _ = fs::remove_file(&path);
                return Ok(file);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(ErrorKind::AlreadyExists, "exhausted Wayland shm temp names"))
}

fn create_shm_file() -> File {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        if let Ok(file) = try_shm_in(Path::new(&dir)) {
            return file;
        }
    }
    try_shm_in(&std::env::temp_dir()).expect("Wayland shm temp file")
}
