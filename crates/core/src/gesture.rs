//! CONTRACT: gesture events emitted by `tidings-gesture`.

/// Which way a gesture drives the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    /// Two fingers landed in the right-edge zone and moved left.
    Open,
    /// Panel was open; two fingers anywhere moved right.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GestureEvent {
    Begin(GestureKind),
    /// Displacement since touchdown in the gesture's direction, as a fraction
    /// of pad width. Unclamped; negative when the user reverses.
    Update {
        progress: f32,
    },
    /// Fingers lifted. `velocity` is pad-widths/s toward completion over the
    /// last ~80 ms.
    End {
        progress: f32,
        velocity: f32,
    },
    /// Third finger or other abort: restore the pre-gesture state.
    Cancel,
}
