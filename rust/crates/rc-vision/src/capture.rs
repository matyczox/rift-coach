//! Game-window capture via Windows Graphics Capture (replaces `src/main/capture.ts` +
//! `src/renderer/capture/index.ts`).
//!
//! - Finds the window by exact title ("League of Legends (TM) Client").
//! - Captures it with WGC (works for occluded windows; reads the window's own surface): no
//!   focus change, no input of any kind is sent to the game.
//! - Returns RGBA frames of the window's CLIENT area (title bar / borders of a windowed game
//!   are cropped away, so calibration rectangles are in client coordinates, as in the TS app
//!   with a borderless game) and the minimap crop from a [`Calibration`].
//!
//! [`WindowCapture`] holds COM/WinRT objects and must stay on the thread that created it;
//! [`CaptureThread`] runs one on a background thread and always keeps the newest frame.
//! The calling process should be per-monitor DPI aware (Tauri apps are), otherwise window
//! coordinates are DPI-virtualised.

use crate::calibration::{Calibration, Rect};
use crate::image::Rgba;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{factory, Interface, HSTRING, PCWSTR};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect, IsIconic, IsWindow};

pub const GAME_WINDOW_TITLE: &str = "League of Legends (TM) Client";

#[derive(Debug)]
pub struct CaptureError(pub String);
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CaptureError {}
impl From<windows::core::Error> for CaptureError {
    fn from(e: windows::core::Error) -> Self {
        CaptureError(format!("{e}"))
    }
}

/// One captured frame of the window's client area.
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    /// Wall-clock ms (UNIX epoch) when the frame was read.
    pub at_ms: u64,
    pub image: Rgba,
}

impl CapturedFrame {
    /// Minimap crop for a calibration measured on some window size (scaled to this frame).
    pub fn minimap(&self, cal: &Calibration) -> Option<Rgba> {
        crop_rect(&self.image, cal.rect_for(self.image.width as i32, self.image.height as i32))
    }
}

/// Crop a rectangle (None when it does not fit inside the image).
pub fn crop_rect(img: &Rgba, r: Rect) -> Option<Rgba> {
    if r.x < 0 || r.y < 0 || r.width <= 8 || r.height <= 8 || (r.x + r.width) as usize > img.width || (r.y + r.height) as usize > img.height {
        return None;
    }
    Some(img.crop(r.x as usize, r.y as usize, r.width as usize, r.height as usize))
}

/// Find a top-level window by exact title.
pub fn find_window(title: &str) -> Option<HWND> {
    let t = HSTRING::from(title);
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), &t) }.ok()?;
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Windows Graphics Capture session on one window (thread-affine).
pub struct WindowCapture {
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    arrived: Arc<AtomicBool>,
    last: Option<CapturedFrame>,
}

impl WindowCapture {
    /// Start capturing the window with this exact title.
    pub fn new(title: &str) -> Result<WindowCapture, CaptureError> {
        let hwnd = find_window(title).ok_or_else(|| CaptureError(format!("window '{title}' not found")))?;
        Self::for_window(hwnd)
    }

    pub fn for_window(hwnd: HWND) -> Result<WindowCapture, CaptureError> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        if !GraphicsCaptureSession::IsSupported()? {
            return Err(CaptureError("Windows Graphics Capture not supported".into()));
        }
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
        }
        let device = device.ok_or_else(|| CaptureError("no D3D11 device".into()))?;
        let context = context.ok_or_else(|| CaptureError("no D3D11 context".into()))?;
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
        let arrived = Arc::new(AtomicBool::new(false));
        let flag = arrived.clone();
        pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(())
        }))?;
        let session = pool.CreateCaptureSession(&item)?;
        // Windows 11: no yellow capture border, no cursor (both optional; older builds lack them).
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;
        Ok(WindowCapture { hwnd, device, context, winrt_device, pool, session, size, staging: None, arrived, last: None })
    }

    /// The window still exists (and is not minimised).
    pub fn alive(&self) -> bool {
        unsafe { IsWindow(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() }
    }

    /// Client-area rectangle relative to the captured surface (window frame bounds).
    fn client_rect_in_capture(&self) -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let mut frame = RECT::default();
            DwmGetWindowAttribute(self.hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32).ok()?;
            let mut cr = RECT::default();
            GetClientRect(self.hwnd, &mut cr).ok()?;
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(self.hwnd, &mut origin).as_bool() {
                return None;
            }
            Some((origin.x - frame.left, origin.y - frame.top, cr.right - cr.left, cr.bottom - cr.top))
        }
    }

    /// Newest frame (waits up to `timeout` for the first one). Returns the previous frame
    /// again when the game produced nothing new.
    pub fn grab(&mut self, timeout: Duration) -> Result<CapturedFrame, CaptureError> {
        let t0 = Instant::now();
        loop {
            // Drain to the newest frame in the pool.
            let mut newest = None;
            while let Ok(f) = self.pool.TryGetNextFrame() {
                newest = Some(f);
            }
            if let Some(frame) = newest {
                self.arrived.store(false, Ordering::Release);
                let size = frame.ContentSize()?;
                if size.Width != self.size.Width || size.Height != self.size.Height {
                    self.size = size;
                    self.pool.Recreate(&self.winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
                }
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
                let img = self.read_texture(&tex, size)?;
                let f = CapturedFrame { at_ms: now_ms(), image: img };
                self.last = Some(f.clone());
                return Ok(f);
            }
            if let Some(l) = &self.last {
                return Ok(l.clone());
            }
            if t0.elapsed() >= timeout {
                return Err(CaptureError("no frame yet".into()));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn read_texture(&mut self, tex: &ID3D11Texture2D, content: SizeInt32) -> Result<Rgba, CaptureError> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut desc) };
        let need_new = !matches!(&self.staging, Some((_, w, h)) if *w == desc.Width && *h == desc.Height);
        if need_new {
            let mut sd = desc;
            sd.Usage = D3D11_USAGE_STAGING;
            sd.BindFlags = 0;
            sd.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            sd.MiscFlags = 0;
            let mut st: Option<ID3D11Texture2D> = None;
            unsafe { self.device.CreateTexture2D(&sd, None, Some(&mut st))? };
            self.staging = Some((st.ok_or_else(|| CaptureError("staging texture".into()))?, desc.Width, desc.Height));
        }
        let (staging, tw, th) = self.staging.clone().unwrap();
        unsafe { self.context.CopyResource(&staging, tex) };
        // Region to read: client area inside the captured surface (whole content if unknown).
        let cw = (content.Width.max(0) as u32).min(tw);
        let ch = (content.Height.max(0) as u32).min(th);
        let (ox, oy, w, h) = match self.client_rect_in_capture() {
            Some((x, y, w, h)) if x >= 0 && y >= 0 && w > 0 && h > 0 && (x + w) as u32 <= cw && (y + h) as u32 <= ch => (x as u32, y as u32, w as u32, h as u32),
            _ => (0, 0, cw, ch),
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let mut out = vec![0u8; (w * h * 4) as usize];
        unsafe {
            let base = mapped.pData as *const u8;
            for y in 0..h {
                let src = std::slice::from_raw_parts(base.add(((y + oy) * mapped.RowPitch + ox * 4) as usize), (w * 4) as usize);
                let dst = &mut out[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            }
            self.context.Unmap(&staging, 0);
        }
        Ok(Rgba { width: w as usize, height: h as usize, data: out })
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureState {
    Off,
    Waiting,
    Connected,
    Error,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub state: CaptureState,
    pub window_width: Option<usize>,
    pub window_height: Option<usize>,
    pub fps: f64,
    pub last_frame_at: Option<u64>,
    pub error: Option<String>,
}

struct Shared {
    latest: Option<CapturedFrame>,
    status: CaptureStatus,
    seq: u64,
}

/// Background capture thread: looks for the game window every 2 s, captures at up to `fps`,
/// keeps only the newest frame. Dropping the handle stops the thread.
pub struct CaptureThread {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl CaptureThread {
    pub fn spawn(title: &str, fps: f64) -> CaptureThread {
        let shared = Arc::new(Mutex::new(Shared { latest: None, status: CaptureStatus { state: CaptureState::Waiting, window_width: None, window_height: None, fps: 0.0, last_frame_at: None, error: None }, seq: 0 }));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st, title) = (shared.clone(), stop.clone(), title.to_string());
        let period = Duration::from_secs_f64(1.0 / fps.clamp(0.5, 60.0));
        let handle = std::thread::Builder::new()
            .name("rc-capture".into())
            .spawn(move || {
                let mut cap: Option<WindowCapture> = None;
                let mut times: Vec<Instant> = Vec::new();
                while !st.load(Ordering::Relaxed) {
                    let tick = Instant::now();
                    if cap.as_ref().is_some_and(|c| !c.alive()) {
                        cap = None;
                    }
                    if cap.is_none() {
                        match WindowCapture::new(&title) {
                            Ok(c) => cap = Some(c),
                            Err(e) => {
                                let mut s = sh.lock().unwrap();
                                s.status = CaptureStatus { state: CaptureState::Waiting, window_width: None, window_height: None, fps: 0.0, last_frame_at: s.status.last_frame_at, error: Some(e.0) };
                                drop(s);
                                sleep_unless(&st, Duration::from_secs(2));
                                continue;
                            }
                        }
                    }
                    let c = cap.as_mut().unwrap();
                    match c.grab(Duration::from_millis(500)) {
                        Ok(f) => {
                            let now = Instant::now();
                            times.push(now);
                            times.retain(|t| now.duration_since(*t) < Duration::from_secs(2));
                            let mut s = sh.lock().unwrap();
                            s.status = CaptureStatus { state: CaptureState::Connected, window_width: Some(f.image.width), window_height: Some(f.image.height), fps: times.len() as f64 / 2.0, last_frame_at: Some(f.at_ms), error: None };
                            s.latest = Some(f);
                            s.seq += 1;
                        }
                        Err(e) => {
                            let mut s = sh.lock().unwrap();
                            s.status.state = CaptureState::Error;
                            s.status.error = Some(e.0);
                            drop(s);
                            cap = None;
                        }
                    }
                    let el = tick.elapsed();
                    if el < period {
                        sleep_unless(&st, period - el);
                    }
                }
            })
            .expect("spawn capture thread");
        CaptureThread { shared, stop, handle: Some(handle) }
    }

    pub fn status(&self) -> CaptureStatus {
        self.shared.lock().unwrap().status.clone()
    }

    /// Newest frame and its sequence number (increments per captured frame).
    pub fn latest(&self) -> Option<(u64, CapturedFrame)> {
        let s = self.shared.lock().unwrap();
        s.latest.clone().map(|f| (s.seq, f))
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20).min(end - Instant::now()));
    }
}

impl Drop for CaptureThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
