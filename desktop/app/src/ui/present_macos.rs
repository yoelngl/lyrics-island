//! macOS: a CALayer sublayer of winit's NSView whose contents is a CGImage of the premultiplied pixels.
//! A sublayer, because AppKit owns the view's own layer contents and redraws it from drawRect:.

use std::ptr;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSColor, NSView};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    CGImageAlphaInfo,
};
use objc2_quartz_core::{kCAGravityTopLeft, CALayer, CATransaction};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tiny_skia::Pixmap;
use winit::window::Window;

pub struct Presenter {
    view: Retained<NSView>,
    layer: Retained<CALayer>,
    space: CFRetained<CGColorSpace>,
}

impl Presenter {
    pub fn new(window: &Window) -> Self {
        let view = match window.window_handle().expect("window handle").as_raw() {
            RawWindowHandle::AppKit(h) => unsafe { Retained::retain(h.ns_view.as_ptr().cast::<NSView>()) }
                .expect("NSView"),
            _ => unreachable!("AppKit window expected"),
        };
        if let Some(win) = view.window() {
            win.setOpaque(false);
            win.setBackgroundColor(Some(&NSColor::clearColor()));
            win.setHasShadow(false);
        }
        view.setWantsLayer(true);
        let layer = CALayer::new();
        layer.setContentsGravity(unsafe { kCAGravityTopLeft });
        layer.setNeedsDisplayOnBoundsChange(false);
        view.layer().expect("layer-backed view").addSublayer(&layer);
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .or_else(CGColorSpace::new_device_rgb)
            .expect("RGB color space");
        Self { view, layer, space }
    }

    pub fn present(&mut self, window: &Window, pm: &Pixmap, _damage: Option<(u32, u32, u32, u32)>) {
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let data = CFData::from_bytes(pm.data());
        let provider = CGDataProvider::with_cf_data(Some(&data));
        let image = unsafe {
            CGImage::new(
                w,
                h,
                8,
                32,
                w * 4,
                Some(&self.space),
                CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
                provider.as_deref(),
                ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        };
        let Some(image) = image else { return };
        CATransaction::begin();
        // Implicit animations would cross-fade every frame.
        CATransaction::setDisableActions(true);
        self.layer.setFrame(self.view.bounds());
        self.layer.setContentsScale(window.scale_factor());
        let contents = CFRetained::as_ptr(&image).as_ptr().cast::<AnyObject>();
        unsafe { self.layer.setContents(Some(&*contents)) };
        CATransaction::commit();
    }
}
