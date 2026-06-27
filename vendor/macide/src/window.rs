//! Cocoa windowing for the Locus macOS IDE (feature `mac-gui`) — multi-window.
//!
//! macOS requires all AppKit work on the main thread, while the IDE logic runs
//! on a worker thread. So the worker never touches `NSWindow` directly: it posts
//! typed [`UiCmd`]s (open / close / set-title) onto a queue and calls [`present`]
//! with a [`DrawBatch`] per window id. A main-thread ~60 Hz timer drains the
//! queue (creating/closing real windows via [`WindowManager`]) and repaints any
//! window whose batch changed. A local `NSEvent` monitor tags each event with
//! the id of the window it came from, translates it via [`crate::events`], and
//! pushes it into the shared [`crate::mailbox`].
//!
//! This mirrors MacNCL's `igui_mac::window`: the side-window model, where the
//! IDE is window id 1 and any extra graphics surface is its own `NSWindow`.
//!
//! Build/run with `--features mac-gui`. Needs a logged-in GUI session.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use block2::RcBlock;
use foreign_types::ForeignType;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask,
    NSEventType, NSImage, NSImageView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSTimer};

use locus_ide_protocol::draw::DrawBatch;
use locus_ide_protocol::event::{MouseButton, MouseOp};

use crate::events as ev;
use crate::mailbox;
use crate::render::CgCanvas;

/// The IDE / main window's id.
pub const MAIN_ID: u64 = 1;

// ── Worker → main-thread command queue ────────────────────────────────

enum UiCmd {
    Open { id: u64, w: f64, h: f64, title: String },
    Close { id: u64 },
    Title { id: u64, title: String },
    /// A file dialog the worker requested; the main thread runs the native panel
    /// and posts the chosen path back under `req`. Native UI must run on the main
    /// thread, so the worker never touches AppKit — it posts this and blocks for
    /// the reply (the protocol's request/response shape).
    FileDialog { req: u64, save: bool, suggested: String },
}

fn cmd_queue() -> &'static Mutex<VecDeque<UiCmd>> {
    static Q: OnceLock<Mutex<VecDeque<UiCmd>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn post(cmd: UiCmd) {
    cmd_queue().lock().unwrap_or_else(|e| e.into_inner()).push_back(cmd);
}

// File-dialog replies: req id → chosen path (None = cancelled). The worker waits
// on the condvar until its req appears.
type DialogReplies = (Mutex<HashMap<u64, Option<String>>>, Condvar);
fn dialog_replies() -> &'static DialogReplies {
    static R: OnceLock<DialogReplies> = OnceLock::new();
    R.get_or_init(|| (Mutex::new(HashMap::new()), Condvar::new()))
}

/// Run a native file dialog from the worker thread. Posts the request to the main
/// thread (which owns AppKit), blocks until it replies, and returns the chosen
/// path (`None` = cancelled). `save` chooses an NSSavePanel vs NSOpenPanel.
fn request_dialog(save: bool, suggested: &str) -> Option<String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let req = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    post(UiCmd::FileDialog { req, save, suggested: suggested.to_string() });
    let (lock, cond) = dialog_replies();
    let mut map = lock.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(path) = map.remove(&req) {
            return path;
        }
        map = cond.wait(map).unwrap_or_else(|e| e.into_inner());
    }
}

/// Native "Open…" panel (worker-callable; blocks until the user chooses/cancels).
pub fn open_file_dialog() -> Option<String> {
    request_dialog(false, "")
}
/// Native "Save As…" panel with a suggested file name (worker-callable; blocks).
pub fn save_file_dialog(suggested: &str) -> Option<String> {
    request_dialog(true, suggested)
}

/// Open a window (worker-callable). It appears on the next main-thread tick.
/// Idempotent per id.
pub fn open_window(id: u64, w: f64, h: f64, title: &str) {
    post(UiCmd::Open { id, w, h, title: title.to_string() });
}
pub fn close_window(id: u64) {
    post(UiCmd::Close { id });
}
pub fn set_window_title(id: u64, title: &str) {
    post(UiCmd::Title { id, title: title.to_string() });
}

// ── Per-window batch store ────────────────────────────────────────────

fn batches() -> &'static Mutex<HashMap<u64, Arc<DrawBatch>>> {
    static B: OnceLock<Mutex<HashMap<u64, Arc<DrawBatch>>>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(HashMap::new()))
}
fn dirty() -> &'static Mutex<HashSet<u64>> {
    static D: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Present a [`DrawBatch`] to window `id` (worker-callable). The main-thread
/// timer renders it on the next tick.
pub fn present(id: u64, batch: DrawBatch) {
    batches()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, Arc::new(batch));
    dirty().lock().unwrap_or_else(|e| e.into_inner()).insert(id);
}

/// Present to the main (IDE) window.
pub fn present_main(batch: DrawBatch) {
    present(MAIN_ID, batch);
}

fn take_batch(id: u64) -> Option<Arc<DrawBatch>> {
    batches().lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
}

// ── Main-thread window registry ───────────────────────────────────────

struct WinEntry {
    window: Retained<NSWindow>,
    view: Retained<NSImageView>,
    w: f64,
    h: f64,
    /// `NSWindow.windowNumber` — a stable per-window integer used to route
    /// `NSEvent`s to the right window (robust integer compare).
    num: isize,
}

struct WindowManager {
    wins: HashMap<u64, WinEntry>,
    mtm: MainThreadMarker,
}

impl WindowManager {
    fn new(mtm: MainThreadMarker) -> Self {
        Self { wins: HashMap::new(), mtm }
    }

    fn open(&mut self, id: u64, w: f64, h: f64, title: &str) {
        if self.wins.contains_key(&id) {
            return;
        }
        let mtm = self.mtm;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str(title));
        window.center();
        let view = NSImageView::new(mtm);
        window.setContentView(Some(&view));
        window.makeKeyAndOrderFront(None);
        let num = window.windowNumber();
        self.wins.insert(id, WinEntry { window, view, w, h, num });
        dirty().lock().unwrap_or_else(|e| e.into_inner()).insert(id);
    }

    fn close(&mut self, id: u64) {
        if let Some(e) = self.wins.remove(&id) {
            e.window.close();
        }
    }

    fn set_title(&mut self, id: u64, title: &str) {
        if let Some(e) = self.wins.get(&id) {
            e.window.setTitle(&NSString::from_str(title));
        }
    }

    /// id of the window an event belongs to, by `NSEvent.windowNumber`. Falls
    /// back to the key (focused) window, then [`MAIN_ID`].
    fn id_for_event(&self, e: &NSEvent) -> u64 {
        let num = e.windowNumber();
        for (id, entry) in &self.wins {
            if entry.num == num {
                return *id;
            }
        }
        for (id, entry) in &self.wins {
            if entry.window.isKeyWindow() {
                return *id;
            }
        }
        MAIN_ID
    }

    fn height_of(&self, id: u64) -> f64 {
        self.wins.get(&id).map(|e| e.h).unwrap_or(0.0)
    }

    fn repaint(&self, id: u64) {
        let Some(entry) = self.wins.get(&id) else { return };
        let Some(batch) = take_batch(id) else { return };
        let scale = entry
            .view
            .window()
            .map(|win| win.backingScaleFactor())
            .filter(|s| *s >= 1.0)
            .unwrap_or(2.0);
        let mut canvas = CgCanvas::new_scaled(entry.w as usize, entry.h as usize, scale);
        canvas.execute(&batch.commands);
        if let Some(path) = std::env::var_os("MACIDE_DUMP") {
            if id == MAIN_ID {
                let _ = std::fs::write(path, canvas.to_ppm());
            }
        }
        let Some(img) = canvas.cg_image() else { return };
        let cg_ref = img.as_ptr();
        // SAFETY: a live CGImageRef from the canvas; `initWithCGImage_size`
        // retains it. We reinterpret the core-graphics CGImage as objc2's.
        let objc_img: &CGImage = unsafe { &*(cg_ref as *const CGImage) };
        let ns_image = NSImage::initWithCGImage_size(
            self.mtm.alloc::<NSImage>(),
            objc_img,
            NSSize::new(entry.w, entry.h),
        );
        entry.view.setImage(Some(&ns_image));
    }

    fn drain_commands(&mut self) {
        let cmds: Vec<UiCmd> =
            cmd_queue().lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        for c in cmds {
            match c {
                UiCmd::Open { id, w, h, title } => self.open(id, w, h, &title),
                UiCmd::Close { id } => self.close(id),
                UiCmd::Title { id, title } => self.set_title(id, &title),
                UiCmd::FileDialog { req, save, suggested } => {
                    let path = run_file_panel(save, &suggested);
                    let (lock, cond) = dialog_replies();
                    lock.lock().unwrap_or_else(|e| e.into_inner()).insert(req, path);
                    cond.notify_all();
                }
            }
        }
    }

    fn repaint_dirty(&self) {
        let ids: Vec<u64> = {
            let mut d = dirty().lock().unwrap_or_else(|e| e.into_inner());
            d.drain().collect()
        };
        for id in ids {
            self.repaint(id);
        }
    }
}

/// Run a native open/save panel. MUST be called on the main thread (it is, from
/// `drain_commands`). Returns the chosen filesystem path, or `None` if cancelled.
fn run_file_panel(save: bool, suggested: &str) -> Option<String> {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new()?;
    unsafe {
        if save {
            let panel = NSSavePanel::savePanel(mtm);
            if !suggested.is_empty() {
                panel.setNameFieldStringValue(&NSString::from_str(suggested));
            }
            if panel.runModal() == NSModalResponseOK {
                return panel.URL().and_then(|u| u.path()).map(|s| s.to_string());
            }
        } else {
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            if panel.runModal() == NSModalResponseOK {
                return panel
                    .URLs()
                    .firstObject()
                    .and_then(|u| u.path())
                    .map(|s| s.to_string());
            }
        }
    }
    None
}

// ── Entry point ───────────────────────────────────────────────────────

/// Open the main (IDE) window and run the AppKit event loop on the calling
/// (main) thread, with `worker` running the IDE logic on a background thread.
/// Returns when the app terminates.
pub fn run<F>(title: &str, width: f64, height: f64, worker: F) -> Result<(), String>
where
    F: FnOnce() + Send + 'static,
{
    run_inner(Some((title.to_string(), width, height)), false, worker)
}

/// Headless-app mode: run the AppKit event loop WITHOUT the IDE main window. The
/// worker opens its own window(s) via [`open_window`]; the process quits when the
/// last window closes. Lets a Locus program ship as a standalone GUI app.
pub fn run_app<F>(worker: F) -> Result<(), String>
where
    F: FnOnce() + Send + 'static,
{
    run_inner(None, true, worker)
}

fn run_inner<F>(
    main_window: Option<(String, f64, f64)>,
    quit_on_last_close: bool,
    worker: F,
) -> Result<(), String>
where
    F: FnOnce() + Send + 'static,
{
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "macide::window::run must be called on the main thread".to_string())?;

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let manager = Rc::new(RefCell::new(WindowManager::new(mtm)));
    if let Some((title, w, h)) = &main_window {
        manager.borrow_mut().open(MAIN_ID, *w, *h, title);
    }

    // Tracks whether any window has ever opened — so `quit_on_last_close` does
    // not terminate during the worker's (windowless) startup, only after a real
    // window has come and gone.
    let had_window = Rc::new(Cell::new(false));

    // Repaint + command-drain timer (~60 Hz) on the main thread.
    let mgr_t = Rc::clone(&manager);
    let app_t = app.clone();
    let had_t = Rc::clone(&had_window);
    let tick = RcBlock::new(move |_t: NonNull<NSTimer>| {
        mgr_t.borrow_mut().drain_commands();
        if quit_on_last_close {
            let empty = mgr_t.borrow().wins.is_empty();
            if !empty {
                had_t.set(true);
            } else if had_t.get() {
                unsafe { app_t.terminate(None) };
            }
        }
        mgr_t.borrow().repaint_dirty();
    });
    let _timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 60.0, true, &tick) };

    // Event monitor: tag each event with its window's id, translate, enqueue.
    let mgr_e = Rc::clone(&manager);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { event.as_ref() };
        let (id, h) = {
            let m = mgr_e.borrow();
            let id = m.id_for_event(e);
            (id, m.height_of(id))
        };
        dispatch_event(e, id, h);
        event.as_ptr()
    });
    let mask = NSEventMask::KeyDown
        | NSEventMask::KeyUp
        | NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::MouseMoved
        | NSEventMask::LeftMouseDragged
        | NSEventMask::ScrollWheel;
    let _monitor =
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };

    app.activate();

    std::thread::Builder::new()
        .name("macide-worker".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(worker)
        .map_err(|e| format!("failed to spawn IDE worker: {e}"))?;

    app.run();
    Ok(())
}

/// Translate one `NSEvent` for window `window_id` and push the resulting
/// `UiEvent`(s) into the mailbox. `view_height` is that window's content height
/// (for the y-flip).
fn dispatch_event(e: &NSEvent, window_id: u64, view_height: f64) {
    let flags = e.modifierFlags().0 as u64;
    let t = e.r#type();

    if t == NSEventType::KeyDown || t == NSEventType::KeyUp {
        let down = t == NSEventType::KeyDown;
        let keycode = e.keyCode();
        let ch = e
            .charactersIgnoringModifiers()
            .and_then(|s| s.to_string().chars().next());
        mailbox::push(window_id, ev::key_event(keycode, ch, flags, down));
        if down {
            if let Some(c) = ch {
                if !c.is_control() {
                    mailbox::push(window_id, ev::char_event(c as u32, flags));
                }
            }
        }
        return;
    }

    let mut push_mouse = |op: MouseOp, button: MouseButton, wheel: i32| {
        let p: NSPoint = e.locationInWindow();
        let y = ev::to_top_left_y(p.y, view_height);
        mailbox::push(
            window_id,
            ev::mouse_event(p.x, y, op, button, flags, wheel, 0),
        );
    };

    match t {
        NSEventType::LeftMouseDown => push_mouse(MouseOp::Down, MouseButton::Left, 0),
        NSEventType::LeftMouseUp => push_mouse(MouseOp::Up, MouseButton::Left, 0),
        NSEventType::RightMouseDown => push_mouse(MouseOp::Down, MouseButton::Right, 0),
        NSEventType::RightMouseUp => push_mouse(MouseOp::Up, MouseButton::Right, 0),
        NSEventType::MouseMoved => push_mouse(MouseOp::Move, MouseButton::None, 0),
        // No distinct Drag op in the protocol; a drag is a Move with the button held.
        NSEventType::LeftMouseDragged => push_mouse(MouseOp::Move, MouseButton::Left, 0),
        NSEventType::ScrollWheel => {
            let dy = e.scrollingDeltaY() as i32;
            push_mouse(MouseOp::Wheel, MouseButton::None, dy);
        }
        _ => {}
    }
}
