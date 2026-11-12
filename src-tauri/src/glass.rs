//! Real macOS 26 Liquid Glass behind the main window's sidebar card, as ~/projects/tauri-glass-proef does it.
//!
//! The page reports where its card is (`set_glass`), and one `NSGlassEffectView` per reported shape sits under the
//! transparent webview, so macOS draws the glass and the page draws the rows on top. For that the window paints an
//! opaque background again and drops the whole-window material of tauri.conf.json (`windowEffects`), which would
//! otherwise show through or double up under the glass.
//!
//! Only the main window gets glass; the menu-bar panel keeps its own material and is never touched. Where the class is
//! missing (macOS before 26, Windows, Linux) or the kill switch is on, nothing changes: `set_glass` answers
//! `supported: false` and the page keeps its CSS glass over the window's material.
//!
//! Kill switch, for a window that misbehaves (tauri-apps/tauri#14454 reports AppKit aborts with WKWebView and
//! NSGlassEffectView on resize): `defaults write dev.sherin.ebmcp NativeGlass -bool false`, or start the app with
//! `EB_MCP_NATIVE_GLASS=0` in its environment. Read once at startup.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::MAIN;

/// The environment variable that turns native glass off for one run (`0`, `false`, `off` or `no`).
const ENV_SWITCH: &str = "EB_MCP_NATIVE_GLASS";
/// The user default (`defaults write dev.sherin.ebmcp NativeGlass -bool false`) that turns it off until removed.
const DEFAULTS_KEY: &str = "NativeGlass";
/// The page reports one card; a few more is fine, many is a mistake on its side.
const MAX_SHAPES: usize = 8;

/// One glass shape, in CSS pixels from the top-left corner of the webview, as `getBoundingClientRect` gives it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GlassRect {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlassAnswer {
    /// Whether native glass is on in this window: macOS 26 or newer, and not switched off.
    pub supported: bool,
    /// How many shapes now have glass behind them.
    pub shapes: usize,
}

/// Whether native glass is on for this run, decided once at startup (`setup`).
pub struct GlassSwitch {
    enabled: bool,
}

/// A shape in AppKit points, in the content view's own coordinates.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

/// Points per CSS pixel: the content view's width in points over the page's `innerWidth` in CSS pixels. At 100 percent
/// page zoom this is 1; at another zoom it is that zoom. `devicePixelRatio` is not used: it also carries the screen's
/// backing scale (2 on Retina), and AppKit frames are in points, not pixels. Anything odd falls back to 1.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn points_per_css_pixel(content_width: f64, viewport_width: f64) -> f64 {
    if !(content_width.is_finite() && viewport_width.is_finite())
        || content_width <= 0.0
        || viewport_width <= 0.0
    {
        return 1.0;
    }
    let scale = content_width / viewport_width;
    if (0.25..=5.0).contains(&scale) {
        scale
    } else {
        1.0
    }
}

/// Where a shape the page reported goes in the content view. The page counts y from the top; an unflipped NSView
/// (AppKit's default) counts it from the bottom. The radius never exceeds half the shorter side.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn frame(rect: &GlassRect, scale: f64, content_height: f64, flipped: bool) -> Frame {
    let (x, top, width, height) = (
        rect.x * scale,
        rect.y * scale,
        rect.width * scale,
        rect.height * scale,
    );
    let y = if flipped {
        top
    } else {
        content_height - top - height
    };
    let radius = (rect.radius * scale).clamp(0.0, width.min(height) / 2.0);
    Frame {
        x,
        y,
        width,
        height,
        radius,
    }
}

/// Only the main window gets glass. The panel is hidden and shown all the time and keeps its own material.
pub fn check_window(label: &str) -> Result<(), String> {
    if label == MAIN {
        Ok(())
    } else {
        Err("Glas kan alleen in het hoofdvenster.".into())
    }
}

/// Refuses what the page should never send: too many shapes, numbers that are not finite, a negative size or radius,
/// an empty id, the same id twice.
pub fn check_rects(rects: &[GlassRect], viewport_width: f64) -> Result<(), String> {
    if rects.len() > MAX_SHAPES {
        return Err(format!(
            "Te veel glasvormen: {} (hooguit {MAX_SHAPES}).",
            rects.len()
        ));
    }
    if !viewport_width.is_finite() || viewport_width < 0.0 {
        return Err("Ongeldige vensterbreedte voor het glas.".into());
    }
    for (index, rect) in rects.iter().enumerate() {
        let numbers = [rect.x, rect.y, rect.width, rect.height, rect.radius];
        if rect.id.is_empty()
            || numbers.iter().any(|n| !n.is_finite())
            || rect.width <= 0.0
            || rect.height <= 0.0
            || rect.radius < 0.0
        {
            return Err(format!("Ongeldige glasvorm: {:?}.", rect.id));
        }
        if rects[..index].iter().any(|other| other.id == rect.id) {
            return Err(format!("Glasvorm {:?} staat er twee keer in.", rect.id));
        }
    }
    Ok(())
}

/// Whether the environment turns native glass off: `0`, `false`, `off` or `no`, in any case.
pub fn switched_off_by_env(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// Decides once whether this run has native glass and, when it does, readies the main window for it: the opaque
/// background and no whole-window material. Older macOS, Windows and a switched-off run keep today's window exactly.
pub fn setup(app: &tauri::App) {
    let supported = platform::supported();
    let env_off = switched_off_by_env(std::env::var(ENV_SWITCH).ok().as_deref());
    let enabled = supported && !env_off && !platform::switched_off_by_defaults();
    if supported && !enabled {
        log::info!("native glass switched off ({ENV_SWITCH} or the {DEFAULTS_KEY} user default)");
    }
    app.manage(GlassSwitch { enabled });
    if !enabled {
        return;
    }
    if let Some(main) = app.get_webview_window(MAIN) {
        // The material from tauri.conf.json (NSVisualEffectView, "sidebar") would cover the opaque background
        // and show under the glass. Cleared at run time, so macOS 13 to 15 and Windows (Mica) keep it.
        if let Err(e) = main.set_effects(None) {
            log::warn!("window material not cleared for native glass: {e}");
        }
        let target = main.clone();
        if let Err(e) = main.run_on_main_thread(move || platform::opaque_background(&target)) {
            log::warn!("opaque window background not set for native glass: {e}");
        }
    }
}

/// Shows exactly these glass shapes in the calling window: new ones are made, known ones moved, the rest removed.
/// `viewport_width` is the page's `innerWidth`, to turn CSS pixels into points at any page zoom.
#[tauri::command]
pub async fn set_glass(
    window: WebviewWindow,
    viewport_width: f64,
    rects: Vec<GlassRect>,
) -> Result<GlassAnswer, String> {
    check_window(window.label())?;
    let enabled = window
        .app_handle()
        .try_state::<GlassSwitch>()
        .is_some_and(|switch| switch.enabled);
    if !enabled {
        return Ok(GlassAnswer {
            supported: false,
            shapes: 0,
        });
    }
    check_rects(&rects, viewport_width)?;
    let (done, answer) = tokio::sync::oneshot::channel();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = done.send(platform::apply(&target, viewport_width, &rects));
        })
        .map_err(|e| format!("Het glas kon niet worden bijgewerkt: {e}"))?;
    let shapes = answer.await.map_err(|_| {
        "Het glas kon niet worden bijgewerkt: het venster antwoordde niet.".to_string()
    })??;
    Ok(GlassAnswer {
        supported: true,
        shapes,
    })
}

/// A page starts loading in the main window: its glass goes, and the new page reports its own shapes.
pub fn page_loading(window: &WebviewWindow) {
    if !enabled(window.app_handle()) {
        return;
    }
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        let _ = platform::apply(&target, 0.0, &[]);
    });
}

/// The main window is gone: its views are forgotten without being messaged, since their window may be freed already.
/// Window events arrive on the main thread, where the views live.
pub fn window_destroyed(app: &AppHandle, label: &str) {
    if enabled(app) {
        platform::forget(label);
    }
}

fn enabled(app: &AppHandle) -> bool {
    app.try_state::<GlassSwitch>()
        .is_some_and(|switch| switch.enabled)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{MainThreadMarker, msg_send, sel};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use tauri::WebviewWindow;

    use super::{DEFAULTS_KEY, GlassRect};

    /// `NSWindowBelow`: under every other subview of the same parent, so under the webview.
    const BELOW: isize = -1;
    /// `NSViewMinYMargin`: in an unflipped view the space below stretches, so a shape stays pinned to the top while
    /// the window is resized natively, until the page reports the new place.
    const PIN_TOP: usize = 8;

    thread_local! {
        /// Main thread only: the glass views per window label, by the id the page gave them. Each entry is a strong
        /// reference of our own, so a view stays alive for as long as it is listed, whatever AppKit does with it.
        static VIEWS: RefCell<HashMap<String, HashMap<String, Retained<AnyObject>>>> = RefCell::new(HashMap::new());
    }

    pub fn supported() -> bool {
        AnyClass::get(c"NSGlassEffectView").is_some()
    }

    /// The `NativeGlass` user default, when it is set and false.
    pub fn switched_off_by_defaults() -> bool {
        let Some(defaults_class) = AnyClass::get(c"NSUserDefaults") else {
            return false;
        };
        let key = NSString::from_str(DEFAULTS_KEY);
        // SAFETY: +standardUserDefaults returns the shared NSUserDefaults (thread-safe, never nil in practice; nil is
        // handled). -objectForKey: takes an NSString and returns an object or nil. -boolValue is only sent to an object
        // that says it responds to it (an NSNumber or NSString), and returns BOOL.
        unsafe {
            let defaults: Option<Retained<AnyObject>> =
                msg_send![defaults_class, standardUserDefaults];
            let Some(defaults) = defaults else {
                return false;
            };
            let value: Option<Retained<AnyObject>> = msg_send![&*defaults, objectForKey: &*key];
            let Some(value) = value else {
                return false;
            };
            let answers: bool = msg_send![&*value, respondsToSelector: sel!(boolValue)];
            if !answers {
                return false;
            }
            let on: bool = msg_send![&*value, boolValue];
            !on
        }
    }

    /// The live NSWindow of a Tauri window, or None when it has none (any more).
    fn ns_window(window: &WebviewWindow) -> Option<&AnyObject> {
        let pointer = window.ns_window().ok()?.cast::<AnyObject>();
        // SAFETY: Tauri hands out the NSWindow it owns for this window; it is either null or a live object for as long
        // as the window exists, and every caller is on the main thread, during a call made for this live window.
        unsafe { pointer.as_ref() }
    }

    /// The window paints its own background again. The transparent window, which the transparent webview needs,
    /// would otherwise show the desktop through the glass.
    pub fn opaque_background(window: &WebviewWindow) {
        if MainThreadMarker::new().is_none() {
            log::warn!("opaque window background asked off the main thread; not set");
            return;
        }
        let (Some(ns_window), Some(color_class)) = (ns_window(window), AnyClass::get(c"NSColor"))
        else {
            return;
        };
        // SAFETY: main thread (checked above). +windowBackgroundColor returns the system's dynamic window colour, which
        // follows the window's appearance. -setBackgroundColor: is NSWindow's setter and takes an NSColor (nil allowed).
        unsafe {
            let color: Option<Retained<AnyObject>> = msg_send![color_class, windowBackgroundColor];
            let _: () = msg_send![ns_window, setBackgroundColor: color.as_deref()];
        }
    }

    /// Makes the window's glass views exactly `rects`. Must run on the main thread; returns how many there are now.
    pub fn apply(
        window: &WebviewWindow,
        viewport_width: f64,
        rects: &[GlassRect],
    ) -> Result<usize, String> {
        if MainThreadMarker::new().is_none() {
            return Err("Het glas kan alleen vanaf de hoofdthread worden bijgewerkt.".into());
        }
        let label = window.label().to_owned();
        let Some(ns_window) = ns_window(window) else {
            forget(&label);
            return Err("Het venster is er niet meer.".into());
        };
        // SAFETY: main thread; -contentView is NSWindow's getter and returns an NSView or nil, retained here.
        let content: Option<Retained<AnyObject>> = unsafe { msg_send![ns_window, contentView] };
        let Some(content) = content else {
            forget(&label);
            return Err("Het venster heeft geen inhoud.".into());
        };
        // SAFETY: main thread; -bounds returns an NSRect and -isFlipped a BOOL, both plain NSView getters on a live view.
        let (bounds, flipped): (NSRect, bool) = unsafe {
            (
                msg_send![&*content, bounds],
                msg_send![&*content, isFlipped],
            )
        };
        let scale = super::points_per_css_pixel(bounds.size.width, viewport_width);

        VIEWS.with(|all| {
            let mut all = all.borrow_mut();
            let views = all.entry(label).or_default();

            // A view the page no longer lists goes; so does one that is no longer in this content view (another
            // content view came, or something took it out): it is made again below.
            views.retain(|id, view| {
                let keep = rects.iter().any(|rect| &rect.id == id) && is_child_of(view, &content);
                if !keep {
                    // SAFETY: main thread; `view` is alive (we hold a strong reference). -removeFromSuperview is a no-op
                    // for a view without a parent.
                    let _: () = unsafe { msg_send![&**view, removeFromSuperview] };
                }
                keep
            });

            for rect in rects {
                let frame = super::frame(rect, scale, bounds.size.height, flipped);
                let ns_frame = NSRect::new(
                    NSPoint::new(frame.x, frame.y),
                    NSSize::new(frame.width, frame.height),
                );
                let view = match views.get(&rect.id) {
                    Some(view) => view.clone(),
                    None => {
                        let view = make_view(&content, ns_frame, flipped)?;
                        views.insert(rect.id.clone(), view.clone());
                        view
                    }
                };
                // SAFETY: main thread; `view` is a live NSGlassEffectView we hold. -setFrame: takes an NSRect.
                let _: () = unsafe { msg_send![&*view, setFrame: ns_frame] };
                set_corner_radius(&view, frame.radius);
            }
            Ok(views.len())
        })
    }

    /// Forgets a window's views: our references are dropped, no message is sent to the views or their window. Main
    /// thread only; anywhere else the thread local is another thread's and holds nothing.
    pub fn forget(label: &str) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let gone = VIEWS.with(|all| all.borrow_mut().remove(label));
        // Dropped here, on the main thread: a view whose parent is gone is freed by AppKit's own release.
        drop(gone);
    }

    fn is_child_of(view: &AnyObject, content: &AnyObject) -> bool {
        // SAFETY: main thread (callers); `view` is alive (held by us). -superview returns the parent or nil.
        let parent: Option<Retained<AnyObject>> = unsafe { msg_send![view, superview] };
        parent.is_some_and(|parent| std::ptr::eq(&*parent, content))
    }

    fn make_view(
        content: &AnyObject,
        frame: NSRect,
        flipped: bool,
    ) -> Result<Retained<AnyObject>, String> {
        let Some(glass_class) = AnyClass::get(c"NSGlassEffectView") else {
            return Err("Dit macOS heeft geen Liquid Glass.".into());
        };
        let nothing: Option<&AnyObject> = None;
        // SAFETY: main thread (callers). +alloc and -initWithFrame: are NSView's designated way to make a view; init
        // returns the view or nil. -addSubview:positioned:relativeTo: takes a view, an NSWindowOrderingMode (NSInteger)
        // and an optional sibling (nil: all of them); the parent retains the view, and we keep our own reference too.
        // -setAutoresizingMask: takes an NSUInteger.
        unsafe {
            let allocated: Allocated<AnyObject> = msg_send![glass_class, alloc];
            let view: Option<Retained<AnyObject>> = msg_send![allocated, initWithFrame: frame];
            let Some(view) = view else {
                return Err("De glasweergave kon niet worden gemaakt.".into());
            };
            let mask: usize = if flipped { 0 } else { PIN_TOP };
            let _: () = msg_send![&*view, setAutoresizingMask: mask];
            let _: () =
                msg_send![content, addSubview: &*view, positioned: BELOW, relativeTo: nothing];
            Ok(view)
        }
    }

    fn set_corner_radius(view: &AnyObject, radius: f64) {
        // SAFETY: main thread (callers); `view` is alive (held by us). -setCornerRadius: is only sent when the view says
        // it responds to it, and takes a CGFloat.
        unsafe {
            let answers: bool = msg_send![view, respondsToSelector: sel!(setCornerRadius:)];
            if answers {
                let _: () = msg_send![view, setCornerRadius: radius];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use tauri::WebviewWindow;

    use super::GlassRect;

    pub fn supported() -> bool {
        false
    }

    pub fn switched_off_by_defaults() -> bool {
        false
    }

    pub fn opaque_background(_window: &WebviewWindow) {}

    pub fn apply(
        _window: &WebviewWindow,
        _viewport_width: f64,
        _rects: &[GlassRect],
    ) -> Result<usize, String> {
        Ok(0)
    }

    pub fn forget(_label: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(x: f64, y: f64, width: f64, height: f64, radius: f64) -> GlassRect {
        GlassRect {
            id: "sidebar".into(),
            x,
            y,
            width,
            height,
            radius,
        }
    }

    #[test]
    fn a_shape_counted_from_the_top_lands_counted_from_the_bottom() {
        // The card 8 from the left, top and bottom of a 780 high window, 202 wide.
        let rect = card(8.0, 8.0, 202.0, 764.0, 18.0);
        assert_eq!(
            frame(&rect, 1.0, 780.0, false),
            Frame {
                x: 8.0,
                y: 8.0,
                width: 202.0,
                height: 764.0,
                radius: 18.0
            }
        );
        // Not symmetric: 20 from the top, 100 high, in a 500 high view.
        let rect = card(8.0, 20.0, 202.0, 100.0, 18.0);
        assert_eq!(frame(&rect, 1.0, 500.0, false).y, 380.0);
        assert_eq!(
            frame(&rect, 1.0, 500.0, true).y,
            20.0,
            "a flipped view counts from the top too"
        );
    }

    #[test]
    fn page_zoom_scales_css_pixels_to_points() {
        // At 125 percent zoom a 1240 point wide window shows 992 CSS pixels.
        let scale = points_per_css_pixel(1240.0, 992.0);
        assert_eq!(scale, 1.25);
        let rect = card(8.0, 8.0, 160.0, 600.0, 18.0);
        assert_eq!(
            frame(&rect, scale, 780.0, false),
            Frame {
                x: 10.0,
                y: 780.0 - 10.0 - 750.0,
                width: 200.0,
                height: 750.0,
                radius: 22.5
            }
        );
        assert_eq!(points_per_css_pixel(1240.0, 1240.0), 1.0);
        assert_eq!(
            points_per_css_pixel(1240.0, 0.0),
            1.0,
            "no width from the page: 1"
        );
        assert_eq!(points_per_css_pixel(1240.0, f64::NAN), 1.0);
        assert_eq!(points_per_css_pixel(1240.0, 1.0), 1.0, "absurd ratios: 1");
    }

    #[test]
    fn the_radius_never_exceeds_half_the_shorter_side() {
        assert_eq!(
            frame(&card(0.0, 0.0, 20.0, 400.0, 18.0), 1.0, 400.0, false).radius,
            10.0
        );
    }

    #[test]
    fn only_the_main_window_gets_glass() {
        assert!(check_window("main").is_ok());
        assert_eq!(
            check_window("panel").unwrap_err(),
            "Glas kan alleen in het hoofdvenster."
        );
        assert!(check_window("").is_err());
    }

    #[test]
    fn odd_shapes_are_refused() {
        assert!(check_rects(&[], 1240.0).is_ok());
        assert!(check_rects(&[card(8.0, 8.0, 202.0, 764.0, 18.0)], 1240.0).is_ok());
        assert!(
            check_rects(&[card(8.0, 8.0, 0.0, 764.0, 18.0)], 1240.0).is_err(),
            "no width"
        );
        assert!(check_rects(&[card(8.0, 8.0, 202.0, 764.0, -1.0)], 1240.0).is_err());
        assert!(check_rects(&[card(f64::NAN, 8.0, 202.0, 764.0, 18.0)], 1240.0).is_err());
        assert!(check_rects(&[card(8.0, 8.0, f64::INFINITY, 764.0, 18.0)], 1240.0).is_err());
        let twice = [
            card(8.0, 8.0, 202.0, 764.0, 18.0),
            card(9.0, 8.0, 202.0, 764.0, 18.0),
        ];
        assert!(check_rects(&twice, 1240.0).is_err());
        let mut nameless = card(8.0, 8.0, 202.0, 764.0, 18.0);
        nameless.id.clear();
        assert!(check_rects(&[nameless], 1240.0).is_err());
        let many: Vec<_> = (0..9)
            .map(|n| GlassRect {
                id: n.to_string(),
                ..card(0.0, 0.0, 1.0, 1.0, 0.0)
            })
            .collect();
        assert!(check_rects(&many, 1240.0).is_err());
        assert!(check_rects(&[], f64::NAN).is_err());
    }

    #[test]
    fn the_environment_can_switch_native_glass_off() {
        for off in ["0", "false", "OFF", " no "] {
            assert!(switched_off_by_env(Some(off)), "{off}");
        }
        for on in [None, Some("1"), Some("true"), Some("")] {
            assert!(!switched_off_by_env(on), "{on:?}");
        }
    }
}
