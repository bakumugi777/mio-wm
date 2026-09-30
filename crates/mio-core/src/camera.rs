use std::{error::Error, fmt};

use crate::{Direction, GeometryError, GridPoint, GridRect, GridSize};

/// The logical camera viewing one rectangle of the shared World.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    position: CameraPosition,
    viewport_size: GridSize,
    zoom: f64,
}

impl Camera {
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn new(position: GridPoint, viewport_size: GridSize) -> Self {
        Self {
            position: CameraPosition {
                x: position.x as f64,
                y: position.y as f64,
            },
            viewport_size,
            zoom: 1.0,
        }
    }

    #[must_use]
    pub const fn position(self) -> CameraPosition {
        self.position
    }

    #[must_use]
    pub const fn viewport_size(self) -> GridSize {
        self.viewport_size
    }

    #[must_use]
    pub const fn zoom(self) -> f64 {
        self.zoom
    }

    /// Changes Camera magnification without changing World geometry.
    ///
    /// # Errors
    /// Returns an error unless `zoom` is finite and greater than zero.
    pub fn set_zoom(&mut self, zoom: f64) -> Result<(), ZoomError> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err(ZoomError);
        }
        self.zoom = zoom;
        Ok(())
    }

    /// Returns the current viewport rectangle.
    ///
    /// # Errors
    /// Returns an error if the viewport exceeds the coordinate model.
    pub fn viewport(self) -> Result<GridRect, GeometryError> {
        let position = self.position.grid_floor()?;
        GridRect::new(
            position.x,
            position.y,
            self.viewport_size.width(),
            self.viewport_size.height(),
        )
    }

    /// Changes the logical camera position.
    ///
    /// # Errors
    /// Returns an error if the viewport would exceed the coordinate model.
    pub fn move_to(&mut self, position: CameraPosition) -> Result<(), CameraPositionError> {
        self.position = position;
        Ok(())
    }

    /// Moves by one full viewport in `direction`.
    ///
    /// # Errors
    /// Returns an error if the resulting viewport exceeds the coordinate model.
    pub fn step(&mut self, direction: Direction) -> Result<(), CameraPositionError> {
        let cells = match direction {
            Direction::Left | Direction::Right => self.viewport_size.width(),
            Direction::Up | Direction::Down => self.viewport_size.height(),
        };
        #[allow(clippy::cast_precision_loss)]
        let cells = cells as f64;
        let (dx, dy) = direction_delta(direction, cells);
        self.move_to(CameraPosition::new(
            self.position.x + dx,
            self.position.y + dy,
        )?)
    }

    /// Changes the viewport grid dimensions without moving its origin.
    ///
    /// # Errors
    /// Returns an error if the resulting viewport exceeds the coordinate model.
    pub fn resize_viewport(&mut self, size: GridSize) -> Result<(), GeometryError> {
        self.position.grid_floor()?;
        self.viewport_size = size;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPosition {
    pub x: f64,
    pub y: f64,
}

impl CameraPosition {
    /// Creates a finite position in continuous World coordinates.
    ///
    /// # Errors
    /// Returns an error if either coordinate is not finite.
    pub fn new(x: f64, y: f64) -> Result<Self, CameraPositionError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(CameraPositionError);
        }
        Ok(Self { x, y })
    }

    #[allow(clippy::cast_precision_loss)]
    fn grid_floor(self) -> Result<GridPoint, GeometryError> {
        if self.x < i64::MIN as f64
            || self.x > i64::MAX as f64
            || self.y < i64::MIN as f64
            || self.y > i64::MAX as f64
        {
            return Err(GeometryError::Overflow);
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(GridPoint::new(self.x.floor() as i64, self.y.floor() as i64))
    }
}

fn direction_delta(direction: Direction, amount: f64) -> (f64, f64) {
    match direction {
        Direction::Left => (-amount, 0.0),
        Direction::Right => (amount, 0.0),
        Direction::Up => (0.0, -amount),
        Direction::Down => (0.0, amount),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CameraPositionError;

impl fmt::Display for CameraPositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("camera position must contain finite coordinates")
    }
}

impl Error for CameraPositionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZoomError;

impl fmt::Display for ZoomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("camera zoom must be finite and greater than zero")
    }
}

impl Error for ZoomError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera::new(GridPoint::new(0, 0), GridSize::new(4, 3).unwrap())
    }

    #[test]
    fn viewport_is_derived_from_position_and_configurable_size() {
        let mut camera = camera();
        camera
            .move_to(CameraPosition::new(-4.0, 6.0).unwrap())
            .unwrap();
        assert_eq!(camera.viewport(), Ok(GridRect::new(-4, 6, 4, 3).unwrap()));
    }

    #[test]
    fn steps_by_one_viewport() {
        let mut camera = camera();
        camera.step(Direction::Right).unwrap();
        camera.step(Direction::Down).unwrap();
        assert_eq!(camera.position(), CameraPosition::new(4.0, 3.0).unwrap());
        camera.step(Direction::Left).unwrap();
        camera.step(Direction::Up).unwrap();
        assert_eq!(camera.position(), CameraPosition::new(0.0, 0.0).unwrap());
    }

    #[test]
    fn zoom_does_not_change_position_or_viewport() {
        let mut camera = camera();
        let viewport = camera.viewport().unwrap();
        camera.set_zoom(0.35).unwrap();
        assert!((camera.zoom() - 0.35).abs() < f64::EPSILON);
        assert_eq!(camera.viewport().unwrap(), viewport);
        assert!(camera.set_zoom(0.0).is_err());
    }
}
