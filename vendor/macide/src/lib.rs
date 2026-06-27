//! `macide` — the Locus macOS IDE shell.
//!
//! The macOS analogue of the Windows `locus-ide` + `igui` stack, built over the
//! UI-neutral [`locus_ide_protocol`] contract. Following MacNCL's split:
//!
//! - [`render`] rasterises the protocol's `DrawCmd` IR via Core Graphics /
//!   Core Text, headlessly (to a `CGBitmapContext`) — **always compiled** and
//!   unit-tested without a display.
//! - [`events`] translates AppKit `NSEvent` integers into protocol `UiEvent`s —
//!   pure functions, **always compiled** and unit-tested.
//! - The AppKit window shell (`NSApplication`/`NSWindow`/`NSView`) lands behind
//!   the `mac-gui` feature, so the default build links no AppKit.
//!
//! The eval/analysis backend reuses the macOS sidecar compiler pipeline (the
//! shared `locus` front end + `locus-llvm` JIT through the macOS libSystem
//! oracle), not the Windows-only `locus-ide` session.

pub mod events;
pub mod mailbox;
pub mod render;

#[cfg(feature = "mac-gui")]
pub mod window;

pub use events::{key_event, mouse_event, resolve_vkey};
pub use mailbox::{WindowEvent};
pub use render::{CgCanvas, TextMetrics, DEFAULT_TEXT_FAMILY};
