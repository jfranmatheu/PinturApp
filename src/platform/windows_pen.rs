#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::Frame;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::Pointer::{
    GetPointerPenInfo, GetPointerTouchInfo, GetPointerType, POINTER_PEN_INFO, POINTER_TOUCH_INFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, POINTER_INPUT_TYPE, PT_PEN, PT_TOUCH, SetWindowLongPtrW,
    WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE, WNDPROC,
};

const NO_PRESSURE_BITS: u32 = u32::MAX;

static HOOKED_HWND: AtomicIsize = AtomicIsize::new(0);
static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static LAST_PRESSURE_BITS: AtomicU32 = AtomicU32::new(NO_PRESSURE_BITS);
static LAST_PRESSURE_TS_MS: AtomicU64 = AtomicU64::new(0);
static PRESSURE_SIGNAL_DETECTED: AtomicBool = AtomicBool::new(false);
static HOOK_INSTALL_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static HOOK_INSTALL_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static ANY_WINDOW_MESSAGE_COUNT: AtomicU64 = AtomicU64::new(0);
static POINTER_MESSAGE_COUNT: AtomicU64 = AtomicU64::new(0);
static PRESSURE_SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_POINTER_TYPE: AtomicU32 = AtomicU32::new(0);

pub fn install(frame: &Frame) {
    HOOK_INSTALL_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let Ok(window_handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get();
    if hwnd == HOOKED_HWND.load(Ordering::Relaxed) {
        return;
    }

    // Install a lightweight WndProc shim to capture WM_POINTER pen pressure.
    let previous = unsafe {
        SetWindowLongPtrW(
            hwnd as HWND,
            GWLP_WNDPROC,
            pen_wndproc as *const () as isize,
        )
    };
    if previous != 0 {
        HOOKED_HWND.store(hwnd, Ordering::Relaxed);
        PREV_WNDPROC.store(previous, Ordering::Relaxed);
        HOOK_INSTALL_SUCCESSES.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn latest_pressure(max_age_ms: u64) -> Option<f32> {
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

pub fn debug_snapshot() -> (u64, u64, u64, u64, u64, u32) {
    (
        HOOK_INSTALL_ATTEMPTS.load(Ordering::Relaxed),
        HOOK_INSTALL_SUCCESSES.load(Ordering::Relaxed),
        ANY_WINDOW_MESSAGE_COUNT.load(Ordering::Relaxed),
        POINTER_MESSAGE_COUNT.load(Ordering::Relaxed),
        PRESSURE_SAMPLE_COUNT.load(Ordering::Relaxed),
        LAST_POINTER_TYPE.load(Ordering::Relaxed),
    )
}

unsafe extern "system" fn pen_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    ANY_WINDOW_MESSAGE_COUNT.fetch_add(1, Ordering::Relaxed);
    if matches!(msg, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
        POINTER_MESSAGE_COUNT.fetch_add(1, Ordering::Relaxed);
        let pointer_id = loword(wparam as u32) as u32;
        let mut pointer_type: POINTER_INPUT_TYPE = 0;
        if unsafe { GetPointerType(pointer_id, &mut pointer_type) } != 0 {
            LAST_POINTER_TYPE.store(pointer_type as u32, Ordering::Relaxed);
            let pressure = if pointer_type == PT_PEN {
                read_pen_pressure(pointer_id)
            } else if pointer_type == PT_TOUCH {
                read_touch_pressure(pointer_id)
            } else {
                read_pen_pressure(pointer_id).or_else(|| read_touch_pressure(pointer_id))
            };
            if let Some(normalized) = pressure {
                LAST_PRESSURE_BITS.store(normalized.to_bits(), Ordering::Relaxed);
                LAST_PRESSURE_TS_MS.store(now_unix_ms(), Ordering::Relaxed);
                PRESSURE_SIGNAL_DETECTED.store(true, Ordering::Relaxed);
                PRESSURE_SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let previous = PREV_WNDPROC.load(Ordering::Relaxed);
    if previous != 0 {
        let previous_proc: WNDPROC = unsafe { std::mem::transmute(previous) };
        unsafe { CallWindowProcW(previous_proc, hwnd, msg, wparam, lparam) }
    } else {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

fn loword(value: u32) -> u16 {
    (value & 0xffff) as u16
}

fn normalize_pressure(raw: u32) -> Option<f32> {
    if raw == 0 {
        None
    } else {
        Some((raw as f32 / 1024.0).clamp(0.0, 1.0))
    }
}

fn read_pen_pressure(pointer_id: u32) -> Option<f32> {
    let mut pen_info = std::mem::MaybeUninit::<POINTER_PEN_INFO>::uninit();
    if unsafe { GetPointerPenInfo(pointer_id, pen_info.as_mut_ptr()) } == 0 {
        return None;
    }
    let pressure_raw = unsafe { pen_info.assume_init().pressure };
    normalize_pressure(pressure_raw)
}

fn read_touch_pressure(pointer_id: u32) -> Option<f32> {
    let mut touch_info = std::mem::MaybeUninit::<POINTER_TOUCH_INFO>::uninit();
    if unsafe { GetPointerTouchInfo(pointer_id, touch_info.as_mut_ptr()) } == 0 {
        return None;
    }
    let pressure_raw = unsafe { touch_info.assume_init().pressure };
    normalize_pressure(pressure_raw)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
