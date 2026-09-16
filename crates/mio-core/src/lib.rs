//! Smithay-independent domain logic for Mio.
//!
//! The world is one. Windows live in it. The display is only a camera looking
//! into that world.

#![forbid(unsafe_code)]

mod action;
mod camera;
mod direction;
mod focus;
mod geometry;
mod grid;
mod output;
mod property;
mod window;
mod world;

pub use action::{Action, ActionOutcome};
pub use camera::{Camera, CameraPosition, CameraPositionError, ZoomError};
pub use direction::{Direction, GridDelta};
pub use focus::Focus;
pub use geometry::{GeometryError, GridPoint, GridRect, GridSize, WorldPoint, WorldRect};
pub use grid::Grid;
pub use output::{OutputError, OutputId};
pub use property::{
    EffectiveWindowProperties, WindowProperty, WindowPropertyKind, WindowPropertySet,
};
pub use window::{GridConstraint, Presentation, Window, WindowId};
pub use world::{World, WorldError};

/// The user-facing project name.
pub const PROJECT_NAME: &str = "Mio";
