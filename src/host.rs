//! Host hooks: the table is shared with WF66 (the Intel box's Forth), so it
//! lives in its own crate, `host/` (f66-host). See design/BOX_PORT.md.

pub use f66_host::*;
