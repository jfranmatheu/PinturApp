#![cfg(target_os = "windows")]

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::Frame;
use libloading::Library;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wintab_lite::{AXIS, DVC, LOGCONTEXT, Packet, WTInfo, WTI, WTClose, WTOpen, WTPKT, WTPacketsGet};

const NO_PRESSURE_BITS: u32 = u32::MAX;

static LAST_PRESSURE_BITS: AtomicU32 = AtomicU32::new(NO_PRESSURE_BITS);
static LAST_PRESSURE_TS_MS: AtomicU64 = AtomicU64::new(0);
static PRESSURE_SIGNAL_DETECTED: AtomicBool = AtomicBool::new(false);
static INIT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static INIT_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static PACKET_POLLS: AtomicU64 = AtomicU64::new(0);
static PACKETS_READ: AtomicU64 = AtomicU64::new(0);
static CONTACT_PACKETS_READ: AtomicU64 = AtomicU64::new(0);
static LAST_HWND: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static BACKEND: RefCell<Option<WintabBackend>> = const { RefCell::new(None) };
}

pub fn install(frame: &Frame) {
    let Ok(window_handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get();
    if hwnd == 0 {
        return;
    }

    BACKEND.with(|cell| {
        let mut slot = cell.borrow_mut();
        let needs_reinit = slot.as_ref().is_none_or(|backend| backend.hwnd != hwnd);
        if !needs_reinit {
            return;
        }

        INIT_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        *slot = WintabBackend::new(hwnd);
        if slot.is_some() {
            INIT_SUCCESSES.fetch_add(1, Ordering::Relaxed);
            LAST_HWND.store(hwnd, Ordering::Relaxed);
        }
    });
}

pub fn latest_pressure(max_age_ms: u64) -> Option<f32> {
    poll_packets();
    let bits = LAST_PRESSURE_BITS.load(Ordering::Relaxed);
    if bits == NO_PRESSURE_BITS {
        return None;
    }
    let now_ms = now_unix_ms();
    let ts_ms = LAST_PRESSURE_TS_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(ts_ms) > max_age_ms {
        return None;
    }
    Some(f32::from_bits(bits).clamp(0.0, 1.0))
}

pub fn pressure_signal_detected() -> bool {
    PRESSURE_SIGNAL_DETECTED.load(Ordering::Relaxed)
}

pub fn debug_snapshot() -> (u64, u64, u64, u64, u64, isize) {
    (
        INIT_ATTEMPTS.load(Ordering::Relaxed),
        INIT_SUCCESSES.load(Ordering::Relaxed),
        PACKET_POLLS.load(Ordering::Relaxed),
        PACKETS_READ.load(Ordering::Relaxed),
        CONTACT_PACKETS_READ.load(Ordering::Relaxed),
        LAST_HWND.load(Ordering::Relaxed),
    )
}

fn poll_packets() {
    BACKEND.with(|cell| {
        if let Some(backend) = cell.borrow_mut().as_mut() {
            backend.poll();
        }
    });
}

struct WintabBackend {
    _library: Library,
    wtclose: WTClose,
    wtpackets_get: WTPacketsGet,
    hctx: *mut wintab_lite::HCTX,
    hwnd: isize,
    pressure_min: i64,
    pressure_max: i64,
}

impl WintabBackend {
    fn new(hwnd: isize) -> Option<Self> {
        let library = unsafe { Library::new("Wintab32.dll").ok()? };
        let wtinfo: WTInfo = unsafe { *library.get::<WTInfo>(b"WTInfoA\0").ok()? };
        let wtopen: WTOpen = unsafe { *library.get::<WTOpen>(b"WTOpenA\0").ok()? };
        let wtclose: WTClose = unsafe { *library.get::<WTClose>(b"WTClose\0").ok()? };
        let wtpackets_get: WTPacketsGet =
            unsafe { *library.get::<WTPacketsGet>(b"WTPacketsGet\0").ok()? };

        let mut log_context = LOGCONTEXT::default();
        let got_context = unsafe {
            wtinfo(
                WTI::DEFSYSCTX,
                0,
                (&mut log_context as *mut LOGCONTEXT).cast::<c_void>(),
            )
        };
        if got_context == 0 {
            return None;
        }

        log_context.lcPktData = WTPKT::all();
        log_context.lcPktMode = WTPKT::BUTTONS;
        let hctx = unsafe { wtopen(hwnd, &mut log_context as *mut LOGCONTEXT, 1) };
        if hctx.is_null() {
            return None;
        }

        let (pressure_min, pressure_max) = read_pressure_range(wtinfo);
        Some(Self {
            _library: library,
            wtclose,
            wtpackets_get,
            hctx,
            hwnd,
            pressure_min,
            pressure_max,
        })
    }

    fn poll(&mut self) {
        PACKET_POLLS.fetch_add(1, Ordering::Relaxed);

        let mut packets = vec![Packet::default(); 32];
        let count = unsafe {
            (self.wtpackets_get)(
                self.hctx,
                packets.len() as i32,
                packets.as_mut_ptr().cast::<c_void>(),
            )
        };
        if count <= 0 {
            return;
        }

        PACKETS_READ.fetch_add(count as u64, Ordering::Relaxed);
        let latest = &packets[(count - 1) as usize];
        let buttons = unsafe { std::ptr::addr_of!(latest.pkButtons).read_unaligned().0 };
        // Ignore hover/mouse packets: only trust pressure while pen tip contact is active.
        if buttons & 0x1 == 0 {
            return;
        }
        CONTACT_PACKETS_READ.fetch_add(1, Ordering::Relaxed);
        let raw = unsafe { std::ptr::addr_of!(latest.pkNormalPressure).read_unaligned() } as i64;
        if let Some(normalized) = normalize_pressure(raw, self.pressure_min, self.pressure_max) {
            LAST_PRESSURE_BITS.store(normalized.to_bits(), Ordering::Relaxed);
            LAST_PRESSURE_TS_MS.store(now_unix_ms(), Ordering::Relaxed);
            PRESSURE_SIGNAL_DETECTED.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for WintabBackend {
    fn drop(&mut self) {
        if !self.hctx.is_null() {
            unsafe {
                (self.wtclose)(self.hctx);
            }
        }
    }
}

fn read_pressure_range(wtinfo: WTInfo) -> (i64, i64) {
    let mut axis = AXIS::default();
    let ok = unsafe {
        wtinfo(
            WTI::DEVICES,
            DVC::NPRESSURE as u32,
            (&mut axis as *mut AXIS).cast::<c_void>(),
        )
    };
    if ok == 0 {
        return (0, 1024);
    }
    let min = axis.axMin as i64;
    let max = axis.axMax as i64;
    if max <= min { (0, 1024) } else { (min, max) }
}

fn normalize_pressure(raw: i64, min: i64, max: i64) -> Option<f32> {
    if max <= min {
        return None;
    }
    let denom = (max - min) as f32;
    if denom <= 0.0 {
        return None;
    }
    let value = ((raw - min) as f32 / denom).clamp(0.0, 1.0);
    if value.is_finite() { Some(value) } else { None }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
