use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum MouseButton {
    None,
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum MouseOp {
    Move,
    Down,
    Up,
    Wheel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub op: MouseOp,
    pub button: MouseButton,
    pub wheel_delta: i32,
    pub modifiers: u32,
    pub time_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum KeyState {
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum UiEvent {
    Close,
    Resize {
        width: u32,
        height: u32,
        dpi: u32,
    },
    TimerTick {
        tick_id: u64,
        time_ms: u64,
    },
    Mouse(MouseEvent),
    Key {
        state: KeyState,
        virtual_key: u32,
        modifiers: u32,
    },
    Char {
        codepoint: u32,
        modifiers: u32,
    },
    Menu {
        item_id: u32,
    },
}
