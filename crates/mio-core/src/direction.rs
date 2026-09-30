/// One of the four directions on Mio's two-dimensional world grid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// Returns a grid delta for `cells` steps in this direction.
    #[must_use]
    pub const fn delta(self, cells: i64) -> GridDelta {
        match self {
            Self::Left => GridDelta { x: -cells, y: 0 },
            Self::Right => GridDelta { x: cells, y: 0 },
            Self::Up => GridDelta { x: 0, y: -cells },
            Self::Down => GridDelta { x: 0, y: cells },
        }
    }
}

/// A signed displacement in grid cells.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GridDelta {
    pub x: i64,
    pub y: i64,
}
