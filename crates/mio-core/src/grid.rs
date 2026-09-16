use crate::{GeometryError, GridPoint, GridRect, GridSize};

/// Mio's logical grid.
///
/// Coordinates and dimensions are already expressed in cells, so the Grid does
/// not store occupancy or duplicate window geometry. It is the common factory
/// for snapped geometry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Grid;

impl Grid {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates a rectangle aligned to whole grid cells.
    ///
    /// # Errors
    /// Returns an error if the rectangle exceeds the geometry model.
    pub fn rect(self, origin: GridPoint, size: GridSize) -> Result<GridRect, GeometryError> {
        GridRect::new(origin.x, origin.y, size.width(), size.height())
    }
}
