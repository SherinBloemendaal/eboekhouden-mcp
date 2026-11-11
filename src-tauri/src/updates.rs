//! Updates, the way Tresso and Varve do them. The macOS app updates through Sparkle: its own windows ask, show the
//! release notes, download, verify the EdDSA signature and relaunch. The framework is bundled
//! (tauri.macos.conf.json) and loaded at runtime, so a development build without it simply has no updater. Windows
//! updates through Tauri's updater plugin, which the web client drives.
//!
//! Before Sparkle relaunches into an update it asks the web client (UPDATE_RELAUNCH_REQUESTED). EB MCP's sessions
//! live only in memory, so the web client asks the user first (now, at quit, or later), signs every session out at
//! e-Boekhouden and waits for running MCP calls; it then answers with `update_relaunch`. Held back, the relaunch waits
//! for that answer, and Sparkle still installs the update when the app quits.

use std::sync::atomic::{AtomicBool, Ordering};

/// Sent to the main window when Sparkle wants to relaunch for an update; the web client answers with
/// `update_relaunch`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const UPDATE_RELAUNCH_REQUESTED: &str = "update-relaunch-requested";

/// Whether the web client listens for UPDATE_RELAUNCH_REQUESTED. Until it does (or when it never loads), nobody could
/// answer, so Sparkle relaunches right away rather than wait forever. Every page load of the main window clears it
/// (lib.rs): a reload has no listener until the app's page says so again.
#[derive(Default)]
pub struct UpdateListener(AtomicBool);

impl UpdateListener {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn is_listening(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// The main window started loading a page.
    pub fn page_loading(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(target_os = "macos")]
mod sparkle {
    use std::cell::{Cell, RefCell};

    use block2::{Block, RcBlock};
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSString};

    /// Asked when Sparkle wants to relaunch: true when the web client was told and answers through
    /// [`continue_relaunch`].
    pub type Ask = Box<dyn Fn() -> bool>;

    struct Sparkle {
        /// The SPUStandardUpdaterController, which lives as long as the app.
        controller: Retained<AnyObject>,
        /// Sparkle keeps its delegate weakly: this keeps it alive.
        _delegate: Retained<Delegate>,
    }

    thread_local! {
      /// Main thread only: the running updater.
      static SPARKLE: RefCell<Option<Sparkle>> = const { RefCell::new(None) };
      /// Main thread only: the block Sparkle gave to continue a relaunch it holds back.
      static RELAUNCH: RefCell<Option<RcBlock<dyn Fn()>>> = const { RefCell::new(None) };
      /// Main thread only: the web client was asked about the held relaunch and has not answered. "Later" answers too,
      /// so a page that loads again takes over only a question nobody answered, never a relaunch put off on purpose.
      static ASKED: Cell<bool> = const { Cell::new(false) };
    }

    #[doc(hidden)]
    pub struct DelegateIvars {
        ask: Ask,
    }

    define_class!(
      // SAFETY: NSObject has no subclassing requirements and Delegate does not implement Drop.
      #[unsafe(super(NSObject))]
      #[thread_kind = MainThreadOnly]
      // Objective-C class names are global: a plain "Delegate" could clash.
      #[name = "EbMcpSparkleUpdaterDelegate"]
      #[ivars = DelegateIvars]
      struct Delegate;

      // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject.
      unsafe impl NSObjectProtocol for Delegate {}

      impl Delegate {
        /// SPUUpdaterDelegate: holds the relaunch back while the web client gets ready; NO (relaunch now) when nobody
        /// can answer.
        // SAFETY: the selector takes the updater, the appcast item and a `void (^)(void)` block, and returns BOOL, as
        // declared here.
        #[unsafe(method(updater:shouldPostponeRelaunchForUpdate:untilInvokingBlock:))]
        fn postpone_relaunch(&self, _updater: &AnyObject, _item: &AnyObject, install: &Block<dyn Fn()>) -> Bool {
          RELAUNCH.with(|slot| *slot.borrow_mut() = Some(install.copy()));
          if (self.ivars().ask)() {
            Bool::YES
          } else {
            RELAUNCH.with(|slot| slot.borrow_mut().take());
            Bool::NO
          }
        }

        /// SPUUpdaterDelegate: the update cycle stopped (an installer error, a cancelled authorization). A relaunch held
        /// from it can no longer continue, so it is forgotten.
        // SAFETY: the selector takes the updater and an NSError, and returns void.
        #[unsafe(method(updater:didAbortWithError:))]
        fn did_abort(&self, _updater: &AnyObject, error: &AnyObject) {
          // "No new update was found" ends a cycle this way too; it says nothing about an update already held.
          if !is_no_update_error(error) {
            RELAUNCH.with(|slot| slot.borrow_mut().take());
          }
        }
      }
    );

    impl Delegate {
        fn new(mtm: MainThreadMarker, ask: Ask) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(DelegateIvars { ask });
            // SAFETY: NSObject's designated initializer on a freshly allocated instance whose ivars are set.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Sparkle's "no new update was found" error (SUSparkleErrorDomain, SUNoUpdateError in SUErrors.h).
    const NO_UPDATE_ERROR: isize = 1001;

    fn is_no_update_error(error: &AnyObject) -> bool {
        let domain = NSString::from_str("SUSparkleErrorDomain");
        // SAFETY: `code` is NSError's NSInteger getter and `domain` its NSString getter; isEqualToString: compares two
        // NSStrings and returns BOOL.
        unsafe {
            let code: isize = msg_send![error, code];
            let error_domain: Retained<NSString> = msg_send![error, domain];
            code == NO_UPDATE_ERROR && error_domain.isEqualToString(&domain)
        }
    }

    /// Loads Sparkle from the app's Frameworks folder and starts its updater, which then checks on its own schedule
    /// (SUEnableAutomaticChecks in Info.plist). Must run on the main thread. Returns whether Sparkle runs.
    pub fn start(ask: Ask) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("Sparkle must start on the main thread");
            return false;
        };
        if running() {
            return true;
        }
        let Some(frameworks) = NSBundle::mainBundle().privateFrameworksPath() else {
            return false;
        };
        let path = NSString::from_str(&format!("{frameworks}/Sparkle.framework"));
        let Some(bundle) = NSBundle::bundleWithPath(&path) else {
            eprintln!("Sparkle is not bundled; this build does not update itself");
            return false;
        };
        // SAFETY: loading a framework the app ships, from its own bundle.
        if !unsafe { bundle.load() } {
            eprintln!("Sparkle is bundled but could not be loaded");
            return false;
        }
        let Some(class) = AnyClass::get(c"SPUStandardUpdaterController") else {
            eprintln!("Sparkle loaded without SPUStandardUpdaterController");
            return false;
        };
        let delegate = Delegate::new(mtm, ask);
        let nil: Option<&AnyObject> = None;
        // SAFETY: -initWithStartingUpdater:updaterDelegate:userDriverDelegate: is Sparkle 2's designated initializer (BOOL,
        // an SPUUpdaterDelegate, an optional SPUStandardUserDriverDelegate) and returns the controller or nil. The delegate
        // implements only optional protocol methods, with the signatures Sparkle declares.
        let controller: Option<Retained<AnyObject>> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![allocated, initWithStartingUpdater: Bool::YES, updaterDelegate: &*delegate, userDriverDelegate: nil]
        };
        let Some(controller) = controller else {
            eprintln!("Sparkle's updater controller did not start");
            return false;
        };
        SPARKLE.with(|slot| {
            *slot.borrow_mut() = Some(Sparkle {
                controller,
                _delegate: delegate,
            })
        });
        true
    }

    /// Opens Sparkle's "Check for Updates" window. Must run on the main thread; does nothing without Sparkle.
    pub fn check() {
        SPARKLE.with(|slot| {
            if let Some(sparkle) = slot.borrow().as_ref() {
                let nil: Option<&AnyObject> = None;
                // SAFETY: -checkForUpdates: is the IBAction behind Sparkle's own "Check for Updates…" menu item.
                let _: () = unsafe { msg_send![&*sparkle.controller, checkForUpdates: nil] };
            }
        });
    }

    pub fn running() -> bool {
        SPARKLE.with(|slot| slot.borrow().is_some())
    }

    /// Sparkle's objects live on the main thread, and so does what this module keeps (thread locals): anywhere else it
    /// would see no updater and no held relaunch. Sync commands run there; this says so loudly should that change.
    pub fn on_main_thread() -> bool {
        let main = MainThreadMarker::new().is_some();
        if !main {
            eprintln!("Sparkle was called off the main thread; the call does nothing");
        }
        main
    }

    /// True while Sparkle holds a relaunch back for the web client's answer (main thread).
    pub fn relaunch_pending() -> bool {
        RELAUNCH.with(|slot| slot.borrow().is_some())
    }

    /// Notes that the web client was asked (true) or answered (false).
    pub fn set_asked(asked: bool) {
        ASKED.with(|cell| cell.set(asked));
    }

    /// A held relaunch the web client was asked about and did not answer (main thread).
    pub fn unanswered() -> bool {
        ASKED.with(Cell::get) && relaunch_pending()
    }

    /// Lets Sparkle install and relaunch the relaunch it holds back; false when none is held.
    pub fn continue_relaunch() -> bool {
        let install = RELAUNCH.with(|slot| slot.borrow_mut().take());
        match install {
            Some(install) => {
                install.call(());
                true
            }
            None => false,
        }
    }
}

/// Asks the web client to get ready for the relaunch Sparkle wants; false when it is not listening, so nobody would
/// answer.
#[cfg(target_os = "macos")]
fn request_relaunch<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::{Emitter, Manager};

    // Only to the main window: the tray panel does not answer update questions.
    let asked = app.state::<UpdateListener>().is_listening()
        && app.emit_to("main", UPDATE_RELAUNCH_REQUESTED, ()).is_ok();
    if asked {
        sparkle::set_asked(true);
    }
    asked
}

/// "Check for Updates…": with an update already downloaded and waiting for the web client, asks it again instead, so
/// the update is one click away rather than downloaded twice. Main thread.
#[cfg(target_os = "macos")]
fn check_or_relaunch<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if !sparkle::on_main_thread() {
        return;
    }
    if !(sparkle::relaunch_pending() && request_relaunch(app)) {
        sparkle::check();
    }
}

/// Starts Sparkle (macOS). Elsewhere the updater plugin is registered in lib.rs instead.
#[cfg(target_os = "macos")]
pub fn setup(app: &tauri::App) {
    // A debug bundle would otherwise offer to replace itself with the latest release.
    if cfg!(debug_assertions) {
        return;
    }
    let handle = app.handle().clone();
    sparkle::start(Box::new(move || request_relaunch(&handle)));
}

/// How this build updates itself: "sparkle" (macOS), "tauri" (the Windows setup) or "none". None covers macOS
/// without Sparkle (a development build) and the .msi: the releases only sign the setup, so an update would install a
/// second copy next to an .msi. The web client shows its own update card only for "tauri", and only when the build
/// carries an updater configuration (release builds; see scripts/release-config.mjs).
#[tauri::command]
pub fn update_channel() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if sparkle::running() {
            "sparkle"
        } else {
            "none"
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::utils::{config::BundleType, platform::bundle_type};
        match bundle_type() {
            Some(BundleType::Nsis) => "tauri",
            _ => "none",
        }
    }
}

/// Opens Sparkle's check window (macOS), for "Zoek naar updates…" in the panel. A sync command, so it runs on the main thread.
#[tauri::command]
pub fn check_for_updates_natively<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    #[cfg(target_os = "macos")]
    check_or_relaunch(&app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// The web client listens for UPDATE_RELAUNCH_REQUESTED from now on. Returns whether Sparkle holds a relaunch whose
/// question went unanswered because the page went away (a reload): the web client then
/// answers that one. A relaunch put off with "Later" stays held until "Check for Updates…". A sync command, so it runs
/// on the main thread, where Sparkle lives.
#[tauri::command]
pub fn update_listening(listener: tauri::State<'_, UpdateListener>) -> bool {
    listener.0.store(true, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    {
        sparkle::on_main_thread() && sparkle::unanswered()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The web client's answer to UPDATE_RELAUNCH_REQUESTED: `proceed` lets Sparkle install and relaunch now; otherwise
/// the relaunch stays held, for "Check for Updates…" to offer again, and the update installs when the app quits.
/// Returns whether a relaunch went ahead. A sync command, so it runs on the main thread, where Sparkle lives.
#[tauri::command]
pub fn update_relaunch(proceed: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        if !sparkle::on_main_thread() {
            return false;
        }
        sparkle::set_asked(false);
        proceed && sparkle::continue_relaunch()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = proceed;
        false
    }
}
