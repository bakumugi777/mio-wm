use std::{error::Error, fmt};

/// A position in the shared world grid.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GridPoint {
    pub x: i64,
    pub y: i64,
}

impl GridPoint {
    #[must_use]
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }

    /// Translates this point, returning an error rather than wrapping at the
    /// finite implementation limits of the otherwise conceptual infinite world.
    ///
    /// # Errors
    /// Returns [`GeometryError::Overflow`] if either coordinate overflows.
    pub fn translated(self, dx: i64, dy: i64) -> Result<Self, GeometryError> {
        Ok(Self {
            x: self.x.checked_add(dx).ok_or(GeometryError::Overflow)?,
            y: self.y.checked_add(dy).ok_or(GeometryError::Overflow)?,
        })
    }
}

/// A continuous position in the shared World, measured in grid-cell units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldPoint {
    pub x: f64,
    pub y: f64,
}

impl WorldPoint {
    /// Creates a finite World position.
    ///
    /// # Errors
    /// Returns [`GeometryError::NonFinite`] for a non-finite coordinate.
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        Ok(Self { x, y })
    }

    /// # Errors
    /// Returns [`GeometryError::NonFinite`] if the result is non-finite.
    pub fn translated(self, dx: f64, dy: f64) -> Result<Self, GeometryError> {
        Self::new(self.x + dx, self.y + dy)
    }
}

impl From<GridPoint> for WorldPoint {
    #[allow(clippy::cast_precision_loss)]
    fn from(value: GridPoint) -> Self {
        Self {
            x: value.x as f64,
            y: value.y as f64,
        }
    }
}

/// A non-empty size measured in grid cells.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GridSize {
    width: u64,
    height: u64,
}

impl GridSize {
    /// Creates a non-empty size.
    ///
    /// # Errors
    /// Returns [`GeometryError::Empty`] for a zero dimension and
    /// [`GeometryError::Overflow`] when a dimension cannot fit the coordinate model.
    pub fn new(width: u64, height: u64) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::Empty);
        }
        if width > i64::MAX as u64 || height > i64::MAX as u64 {
            return Err(GeometryError::Overflow);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u64 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u64 {
        self.height
    }
}

/// A half-open rectangle in grid cells: `[x, right) × [y, bottom)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GridRect {
    x: i64,
    y: i64,
    width: u64,
    height: u64,
}

/// A Window rectangle in the continuous World. Its size remains expressed in
/// grid cells; only the Grid constraint determines whether its origin must be
/// aligned to cell boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldRect {
    origin: WorldPoint,
    size: GridSize,
}

impl WorldRect {
    /// # Errors
    /// Returns an error if the origin is non-finite.
    pub fn new(origin: WorldPoint, size: GridSize) -> Result<Self, GeometryError> {
        // Revalidate public fields in case this type later gains unchecked constructors.
        Ok(Self {
            origin: WorldPoint::new(origin.x, origin.y)?,
            size,
        })
    }

    #[must_use]
    pub const fn origin(self) -> WorldPoint {
        self.origin
    }
    #[must_use]
    pub const fn x(self) -> f64 {
        self.origin.x
    }
    #[must_use]
    pub const fn y(self) -> f64 {
        self.origin.y
    }
    #[must_use]
    pub const fn width(self) -> u64 {
        self.size.width()
    }
    #[must_use]
    pub const fn height(self) -> u64 {
        self.size.height()
    }
    #[must_use]
    pub const fn size(self) -> GridSize {
        self.size
    }
    /// # Errors
    /// Returns an error if the edge is outside the finite World model.
    #[allow(clippy::cast_precision_loss)]
    pub fn right(self) -> Result<f64, GeometryError> {
        let value = self.x() + self.width() as f64;
        value
            .is_finite()
            .then_some(value)
            .ok_or(GeometryError::Overflow)
    }
    /// # Errors
    /// Returns an error if the edge is outside the finite World model.
    #[allow(clippy::cast_precision_loss)]
    pub fn bottom(self) -> Result<f64, GeometryError> {
        let value = self.y() + self.height() as f64;
        value
            .is_finite()
            .then_some(value)
            .ok_or(GeometryError::Overflow)
    }
    /// # Errors
    /// Returns an error if the origin is non-finite.
    pub fn moved_to(self, origin: WorldPoint) -> Result<Self, GeometryError> {
        Self::new(origin, self.size)
    }
    /// # Errors
    /// Returns an error if the translated origin is non-finite.
    pub fn translated(self, dx: f64, dy: f64) -> Result<Self, GeometryError> {
        self.moved_to(self.origin.translated(dx, dy)?)
    }
    /// # Errors
    /// Returns an error if the resulting rectangle is invalid.
    pub fn resized(self, size: GridSize) -> Result<Self, GeometryError> {
        Self::new(self.origin, size)
    }
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.x() < other.right().unwrap_or(f64::INFINITY)
            && other.x() < self.right().unwrap_or(f64::INFINITY)
            && self.y() < other.bottom().unwrap_or(f64::INFINITY)
            && other.y() < self.bottom().unwrap_or(f64::INFINITY)
    }
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x().max(other.x());
        let top = self.y().max(other.y());
        let right = self.right().ok()?.min(other.right().ok()?);
        let bottom = self.bottom().ok()?.min(other.bottom().ok()?);
        if left >= right || top >= bottom {
            return None;
        }
        // Clip rectangles are used for visibility only. Window sizes are integral,
        // while a fractional origin may yield a fractional clip; retain a containing
        // cell size here rather than creating a second semantic Window geometry.
        let width = (right - left).ceil();
        let height = (bottom - top).ceil();
        if width > u64::MAX as f64 || height > u64::MAX as f64 {
            return None;
        }
        Self::new(
            WorldPoint::new(left, top).ok()?,
            GridSize::new(width as u64, height as u64).ok()?,
        )
        .ok()
    }
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn center(self) -> (f64, f64) {
        (
            self.x() + self.width() as f64 / 2.0,
            self.y() + self.height() as f64 / 2.0,
        )
    }
}

impl From<GridRect> for WorldRect {
    fn from(value: GridRect) -> Self {
        Self {
            origin: value.origin().into(),
            size: value.size(),
        }
    }
}

impl PartialEq<GridRect> for WorldRect {
    fn eq(&self, other: &GridRect) -> bool {
        *self == WorldRect::from(*other)
    }
}

impl PartialEq<WorldRect> for GridRect {
    fn eq(&self, other: &WorldRect) -> bool {
        WorldRect::from(*self) == *other
    }
}

impl GridRect {
    /// Creates a non-empty rectangle whose right and bottom edges fit in `i64`.
    ///
    /// # Errors
    /// Returns an error for empty dimensions or an overflowing edge.
    pub fn new(x: i64, y: i64, width: u64, height: u64) -> Result<Self, GeometryError> {
        let size = GridSize::new(width, height)?;
        let rect = Self {
            x,
            y,
            width: size.width,
            height: size.height,
        };
        rect.right()?;
        rect.bottom()?;
        Ok(rect)
    }

    #[must_use]
    pub const fn origin(self) -> GridPoint {
        GridPoint::new(self.x, self.y)
    }

    #[must_use]
    pub const fn x(self) -> i64 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i64 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> u64 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u64 {
        self.height
    }

    #[must_use]
    pub const fn size(self) -> GridSize {
        GridSize {
            width: self.width,
            height: self.height,
        }
    }

    /// # Errors
    /// Returns an error if the right edge exceeds `i64`.
    pub fn right(self) -> Result<i64, GeometryError> {
        self.x
            .checked_add(i64::try_from(self.width).map_err(|_| GeometryError::Overflow)?)
            .ok_or(GeometryError::Overflow)
    }

    /// # Errors
    /// Returns an error if the bottom edge exceeds `i64`.
    pub fn bottom(self) -> Result<i64, GeometryError> {
        self.y
            .checked_add(i64::try_from(self.height).map_err(|_| GeometryError::Overflow)?)
            .ok_or(GeometryError::Overflow)
    }

    /// # Errors
    /// Returns an error if the moved rectangle exceeds the coordinate model.
    pub fn moved_to(self, origin: GridPoint) -> Result<Self, GeometryError> {
        Self::new(origin.x, origin.y, self.width, self.height)
    }

    /// # Errors
    /// Returns an error if translation overflows the coordinate model.
    pub fn translated(self, dx: i64, dy: i64) -> Result<Self, GeometryError> {
        self.moved_to(self.origin().translated(dx, dy)?)
    }

    /// # Errors
    /// Returns an error if the resized rectangle exceeds the coordinate model.
    pub fn resized(self, size: GridSize) -> Result<Self, GeometryError> {
        Self::new(self.x, self.y, size.width, size.height)
    }

    /// Returns true when the interiors of the rectangles overlap.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        let self_right = i128::from(self.x) + i128::from(self.width);
        let self_bottom = i128::from(self.y) + i128::from(self.height);
        let other_right = i128::from(other.x) + i128::from(other.width);
        let other_bottom = i128::from(other.y) + i128::from(other.height);
        i128::from(self.x) < other_right
            && i128::from(other.x) < self_right
            && i128::from(self.y) < other_bottom
            && i128::from(other.y) < self_bottom
    }

    /// Returns the visible intersection of two rectangles.
    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (i128::from(self.x) + i128::from(self.width))
            .min(i128::from(other.x) + i128::from(other.width));
        let bottom = (i128::from(self.y) + i128::from(self.height))
            .min(i128::from(other.y) + i128::from(other.height));
        if i128::from(left) >= right || i128::from(top) >= bottom {
            return None;
        }
        let width = u64::try_from(right - i128::from(left)).ok()?;
        let height = u64::try_from(bottom - i128::from(top)).ok()?;
        Self::new(left, top, width, height).ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    Empty,
    Overflow,
    NonFinite,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("grid rectangles and sizes must be non-empty"),
            Self::Overflow => formatter.write_str("grid geometry exceeds the supported i64 world"),
            Self::NonFinite => formatter.write_str("World coordinates must be finite"),
        }
    }
}

impl Error for GeometryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_edges_do_not_overlap() {
        let left = GridRect::new(-2, 0, 2, 2).unwrap();
        let right = GridRect::new(0, 0, 2, 2).unwrap();
        assert!(!left.overlaps(right));
        assert_eq!(left.intersection(right), None);
    }

    #[test]
    fn intersection_handles_negative_coordinates() {
        let a = GridRect::new(-4, -3, 5, 4).unwrap();
        let b = GridRect::new(-2, -2, 5, 5).unwrap();
        assert_eq!(
            a.intersection(b),
            Some(GridRect::new(-2, -2, 3, 3).unwrap())
        );
    }

    #[test]
    fn rejects_empty_and_overflowing_rectangles() {
        assert_eq!(GridRect::new(0, 0, 0, 1), Err(GeometryError::Empty));
        assert_eq!(
            GridRect::new(i64::MAX, 0, 1, 1),
            Err(GeometryError::Overflow)
        );
    }
}
