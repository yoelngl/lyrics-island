//! Windows: per-pixel alpha via WS_EX_LAYERED + UpdateLayeredWindow from a top-down 32-bit DIB section.

use std::ffi::c_void;
use std::ptr;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tiny_skia::Pixmap;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
    HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, SetWindowLongW, UpdateLayeredWindow, GWL_EXSTYLE, ULW_ALPHA, WS_EX_LAYERED,
};
use winit::window::Window;

pub struct Presenter {
    hwnd: HWND,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    size: (u32, u32),
}

impl Presenter {
    pub fn new(window: &Window) -> Self {
        let hwnd = match window.window_handle().expect("window handle").as_raw() {
            RawWindowHandle::Win32(h) => HWND(h.hwnd.get() as *mut c_void),
            _ => unreachable!("Win32 window expected"),
        };
        let dc = unsafe { CreateCompatibleDC(None) };
        Self { hwnd, dc, bmp: HBITMAP::default(), old: HGDIOBJ::default(), bits: ptr::null_mut(), size: (0, 0) }
    }

    pub fn present(&mut self, _window: &Window, pm: &Pixmap) {
        let (w, h) = (pm.width(), pm.height());
        unsafe {
            // winit rewrites GWL_EXSTYLE whenever its window flags change, dropping WS_EX_LAYERED.
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE);
            if ex & WS_EX_LAYERED.0 as i32 == 0 {
                SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as i32);
            }
            if self.size != (w, h) || self.bits.is_null() {
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w as i32,
                        biHeight: -(h as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits: *mut c_void = ptr::null_mut();
                let Ok(bmp) = CreateDIBSection(Some(self.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                    return;
                };
                let prev = SelectObject(self.dc, bmp.into());
                if self.bmp.is_invalid() {
                    self.old = prev;
                } else {
                    let _ = DeleteObject(self.bmp.into());
                }
                self.bmp = bmp;
                self.bits = bits.cast();
                self.size = (w, h);
            }
            let dst = std::slice::from_raw_parts_mut(self.bits, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
            let size = SIZE { cx: w as i32, cy: h as i32 };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                Some(&raw const size),
                Some(self.dc),
                Some(&raw const src),
                COLORREF(0),
                Some(&raw const blend),
                ULW_ALPHA,
            );
        }
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp.into());
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
