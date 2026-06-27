//! The IDE event mailbox — a thread-safe FIFO of `(window_id, UiEvent)`.
//!
//! macOS requires all AppKit work on the main thread, while the IDE's logic
//! (editor, REPL, eval) runs on a worker thread. The AppKit `NSEvent` monitor
//! (`window.rs`, main thread) translates events via [`crate::events`] and
//! [`push`]es them here; the worker drains them with [`try_next`]/[`drain`].
//! This is the macOS analogue of MacNCL's `igui_events` mailbox and decouples
//! the UI thread from the worker — the "event routing" seam of the IDE.
//!
//! Kept free of any AppKit/objc2 types so it is **always compiled** and
//! unit-testable without a display.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use locus_ide_protocol::event::UiEvent;

/// An event tagged with the window (pane) it was delivered to.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowEvent {
    pub window_id: u64,
    pub event: UiEvent,
}

fn queue() -> &'static Mutex<VecDeque<WindowEvent>> {
    static Q: OnceLock<Mutex<VecDeque<WindowEvent>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Push an event for `window_id` (called from the AppKit event monitor).
pub fn push(window_id: u64, event: UiEvent) {
    queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(WindowEvent { window_id, event });
}

/// Pop the oldest pending event, if any (non-blocking; called by the worker).
pub fn try_next() -> Option<WindowEvent> {
    queue().lock().unwrap_or_else(|e| e.into_inner()).pop_front()
}

/// Drain all pending events in arrival order (called by the worker per tick).
pub fn drain() -> Vec<WindowEvent> {
    let mut q = queue().lock().unwrap_or_else(|e| e.into_inner());
    q.drain(..).collect()
}

/// Number of events currently queued.
pub fn len() -> usize {
    queue().lock().unwrap_or_else(|e| e.into_inner()).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_ide_protocol::event::{KeyState, UiEvent};

    #[test]
    fn push_then_drain_is_fifo() {
        // Use a dedicated window id so this test is independent of others that
        // share the process-global queue.
        const W: u64 = 0xF1F0;
        push(W, UiEvent::Close);
        push(
            W,
            UiEvent::Key { state: KeyState::Down, virtual_key: 0x41, modifiers: 0 },
        );
        let mine: Vec<_> = drain().into_iter().filter(|e| e.window_id == W).collect();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].event, UiEvent::Close);
        assert!(matches!(mine[1].event, UiEvent::Key { virtual_key: 0x41, .. }));
    }

    #[test]
    fn try_next_returns_pushed_event() {
        const W: u64 = 0xF1F1;
        push(W, UiEvent::Resize { width: 800, height: 600, dpi: 96 });
        // Drain any unrelated events first so we deterministically find ours.
        let found = std::iter::from_fn(try_next)
            .find(|e| e.window_id == W)
            .expect("our event");
        assert_eq!(found.event, UiEvent::Resize { width: 800, height: 600, dpi: 96 });
    }
}
