use mio_core::{Camera, WorldRect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Insets a presented Window without changing its World geometry.
///
/// The inset is clamped independently on each axis so even a very small
/// presentation remains at least one logical pixel wide and high.
pub fn inset_screen_rect(rect: ScreenRect, gap: u32) -> ScreenRect {
    let requested = i32::try_from(gap).unwrap_or(i32::MAX);
    let horizontal = requested.min(rect.width.saturating_sub(1) / 2);
    let vertical = requested.min(rect.height.saturating_sub(1) / 2);
    ScreenRect {
        x: rect.x.saturating_add(horizontal),
        y: rect.y.saturating_add(vertical),
        width: rect.width.saturating_sub(horizontal.saturating_mul(2)),
        height: rect.height.saturating_sub(vertical.saturating_mul(2)),
    }
}

/// Converts Core grid cells to logical output pixels. Camera movement changes
/// only the origin subtraction; it never changes the supplied World rectangle.
#[allow(clippy::cast_precision_loss)]
pub fn world_to_screen<R: Into<WorldRect>>(
    rect: R,
    camera: Camera,
    output_size: (i32, i32),
) -> Option<ScreenRect> {
    let rect = rect.into();
    let viewport = camera.viewport_size();
    let scale = |value: f64, pixels: i32, cells: u64| {
        MeasureInput::new(value, pixels, cells, camera.zoom()).resolve()
    };

    Some(ScreenRect {
        x: scale(
            rect.x() as f64 - camera.position().x,
            output_size.0,
            viewport.width(),
        )?
        .saturating_add(zoom_center_offset(output_size.0, camera.zoom())),
        y: scale(
            rect.y() as f64 - camera.position().y,
            output_size.1,
            viewport.height(),
        )?
        .saturating_add(zoom_center_offset(output_size.1, camera.zoom())),
        width: scale(rect.width() as f64, output_size.0, viewport.width())?.max(1),
        height: scale(rect.height() as f64, output_size.1, viewport.height())?.max(1),
    })
}

fn zoom_center_offset(pixels: i32, zoom: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let offset = (f64::from(pixels) * (1.0 - zoom) / 2.0).round() as i32;
    offset
}

struct MeasureInput {
    value: f64,
    pixels: f64,
    cells: f64,
    zoom: f64,
}

impl MeasureInput {
    #[allow(clippy::cast_precision_loss)]
    fn new(value: f64, pixels: i32, cells: u64, zoom: f64) -> Self {
        Self {
            value,
            pixels: f64::from(pixels),
            cells: cells as f64,
            zoom,
        }
    }

    fn resolve(self) -> Option<i32> {
        let scaled = (self.value * self.pixels / self.cells * self.zoom).round();
        if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        Some(scaled as i32)
    }
}

#[cfg(test)]
mod tests {
    use mio_core::{Camera, CameraPosition, Direction, GridPoint, GridRect, GridSize};

    use super::*;

    #[test]
    fn camera_transform_preserves_world_geometry() {
        let mut camera = Camera::new(GridPoint::new(0, 0), GridSize::new(4, 4).unwrap());
        let world_rect = GridRect::new(3, 1, 3, 2).unwrap();

        assert_eq!(
            world_to_screen(world_rect, camera, (800, 600)),
            Some(ScreenRect {
                x: 600,
                y: 150,
                width: 600,
                height: 300,
            })
        );
        camera.step(Direction::Right).unwrap();
        assert_eq!(
            world_to_screen(world_rect, camera, (800, 600)),
            Some(ScreenRect {
                x: -200,
                y: 150,
                width: 600,
                height: 300,
            })
        );
        assert_eq!(world_rect, GridRect::new(3, 1, 3, 2).unwrap());
    }

    #[test]
    fn negative_world_coordinates_transform_relative_to_camera() {
        let camera = Camera::new(GridPoint::new(-4, -4), GridSize::new(4, 4).unwrap());
        let rect = GridRect::new(-3, -2, 1, 1).unwrap();
        assert_eq!(
            world_to_screen(rect, camera, (1000, 800)),
            Some(ScreenRect {
                x: 250,
                y: 400,
                width: 250,
                height: 200,
            })
        );
    }

    #[test]
    fn zoom_changes_only_the_camera_transform() {
        let mut camera = Camera::new(GridPoint::new(0, 0), GridSize::new(4, 4).unwrap());
        camera.set_zoom(0.5).unwrap();
        let rect = GridRect::new(2, 1, 2, 2).unwrap();
        assert_eq!(
            world_to_screen(rect, camera, (800, 600)),
            Some(ScreenRect {
                x: 400,
                y: 225,
                width: 200,
                height: 150,
            })
        );
        assert_eq!(rect, GridRect::new(2, 1, 2, 2).unwrap());
    }

    #[test]
    fn fractional_camera_position_centers_odd_sized_window() {
        let mut camera = Camera::new(GridPoint::new(0, 0), GridSize::new(4, 4).unwrap());
        camera
            .move_to(CameraPosition::new(-0.5, -1.0).unwrap())
            .unwrap();
        let rect = GridRect::new(0, 0, 3, 2).unwrap();
        let screen = world_to_screen(rect, camera, (800, 600)).unwrap();
        assert_eq!(screen.x + screen.width / 2, 400);
        assert_eq!(screen.y + screen.height / 2, 300);
    }

    #[test]
    fn outer_gap_insets_without_changing_the_center() {
        assert_eq!(
            inset_screen_rect(
                ScreenRect {
                    x: -20,
                    y: 10,
                    width: 100,
                    height: 60,
                },
                8,
            ),
            ScreenRect {
                x: -12,
                y: 18,
                width: 84,
                height: 44,
            }
        );
    }

    #[test]
    fn outer_gap_keeps_tiny_presentations_non_empty() {
        assert_eq!(
            inset_screen_rect(
                ScreenRect {
                    x: 4,
                    y: 7,
                    width: 2,
                    height: 1,
                },
                u32::MAX,
            ),
            ScreenRect {
                x: 4,
                y: 7,
                width: 2,
                height: 1,
            }
        );
    }
}
