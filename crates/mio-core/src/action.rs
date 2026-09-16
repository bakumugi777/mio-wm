use crate::{
    Direction, GridPoint, GridRect, GridSize, OutputId, WindowId, WindowProperty,
    WindowPropertyKind, WorldPoint, WorldRect,
};

/// A shared user-facing operation independent of its keyboard, pointer, IPC, or
/// future programmable origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Focus(Direction),
    FocusWindow(WindowId),
    MoveWindow {
        id: WindowId,
        origin: GridPoint,
    },
    MoveWindowContinuous {
        id: WindowId,
        origin: WorldPoint,
    },
    ResizeWindow {
        id: WindowId,
        size: GridSize,
    },
    ResizeWindowRect {
        id: WindowId,
        rect: GridRect,
    },
    ResizeWindowContinuousRect {
        id: WindowId,
        rect: WorldRect,
    },
    SetNextPlacement(Direction),
    ActivateOutput(OutputId),
    CycleOutput,
    CameraStep(Direction),
    CameraNudge(Direction),
    CameraPan {
        delta_x: f64,
        delta_y: f64,
    },
    CameraTo(WindowId),
    CameraCenter(WindowId),
    CameraFollow(WindowId),
    CameraZoom(f64),
    ToggleFloating(WindowId),
    SetWindowProperty {
        id: WindowId,
        property: WindowProperty,
    },
    ClearWindowProperty {
        id: WindowId,
        kind: WindowPropertyKind,
    },
    ToggleFullscreen(WindowId),
    ToggleMaximized(WindowId),
    CloseWindow(WindowId),
}

/// Information an adapter may need after Core applies an [`Action`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionOutcome {
    Applied,
    FocusChanged(Option<WindowId>),
    CloseRequested(WindowId),
}
