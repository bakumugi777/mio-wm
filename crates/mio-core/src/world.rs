use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{
    Action, ActionOutcome, Camera, CameraPosition, CameraPositionError, Direction, Focus,
    GeometryError, Grid, GridConstraint, GridPoint, GridRect, GridSize, OutputError, OutputId,
    Presentation, Window, WindowId, WindowProperty, WindowPropertyKind, WorldPoint, WorldRect,
    ZoomError,
};

/// The single source of truth for logical windows, focus, and the camera.
#[derive(Debug)]
pub struct World {
    windows: BTreeMap<WindowId, Window>,
    grid: Grid,
    focus: Focus,
    camera: Camera,
    active_output: OutputId,
    inactive_cameras: BTreeMap<OutputId, Camera>,
    next_output_id: u64,
    next_placement: Option<(Direction, Option<WindowId>)>,
    next_window_id: u64,
}

impl World {
    #[must_use]
    pub fn new(camera: Camera) -> Self {
        Self {
            windows: BTreeMap::new(),
            grid: Grid::new(),
            focus: Focus::default(),
            camera,
            active_output: OutputId::from_raw(1),
            inactive_cameras: BTreeMap::new(),
            next_output_id: 2,
            next_placement: None,
            next_window_id: 1,
        }
    }

    #[must_use]
    pub fn windows(&self) -> impl ExactSizeIterator<Item = &Window> {
        self.windows.values()
    }

    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    #[must_use]
    pub const fn focused(&self) -> Option<WindowId> {
        self.focus.window()
    }

    #[must_use]
    pub const fn grid(&self) -> Grid {
        self.grid
    }

    #[must_use]
    pub const fn focus(&self) -> Focus {
        self.focus
    }

    #[must_use]
    pub const fn camera(&self) -> &Camera {
        &self.camera
    }

    #[must_use]
    pub const fn active_output(&self) -> OutputId {
        self.active_output
    }

    pub fn output_cameras(&self) -> impl Iterator<Item = (OutputId, &Camera)> {
        std::iter::once((self.active_output, &self.camera)).chain(
            self.inactive_cameras
                .iter()
                .map(|(&id, camera)| (id, camera)),
        )
    }

    #[must_use]
    pub fn camera_for_output(&self, id: OutputId) -> Option<&Camera> {
        if id == self.active_output {
            Some(&self.camera)
        } else {
            self.inactive_cameras.get(&id)
        }
    }

    /// Adds another view into the same World without copying any Window state.
    ///
    /// # Errors
    /// Returns [`OutputError::IdExhausted`] when no stable ID remains.
    pub fn add_output(&mut self, camera: Camera) -> Result<OutputId, OutputError> {
        let id = OutputId::from_raw(self.next_output_id);
        self.next_output_id = self
            .next_output_id
            .checked_add(1)
            .ok_or(OutputError::IdExhausted)?;
        self.inactive_cameras.insert(id, camera);
        Ok(id)
    }

    /// Selects which Output receives Camera Actions and new Window placement.
    ///
    /// # Errors
    /// Returns [`OutputError::Unknown`] for an absent Output.
    pub fn activate_output(&mut self, id: OutputId) -> Result<(), OutputError> {
        if id == self.active_output {
            return Ok(());
        }
        let camera = self
            .inactive_cameras
            .remove(&id)
            .ok_or(OutputError::Unknown(id))?;
        let previous_id = self.active_output;
        let previous_camera = std::mem::replace(&mut self.camera, camera);
        self.inactive_cameras.insert(previous_id, previous_camera);
        self.active_output = id;
        Ok(())
    }

    /// Activates the next stable Output ID, wrapping at the end.
    pub fn cycle_output(&mut self) {
        let next = self
            .inactive_cameras
            .keys()
            .copied()
            .find(|id| *id > self.active_output)
            .or_else(|| self.inactive_cameras.keys().next().copied());
        if let Some(next) = next {
            // The ID came directly from inactive_cameras.
            let _ = self.activate_output(next);
        }
    }

    /// Adds an already-positioned window and returns its stable ID.
    ///
    /// # Errors
    /// Returns [`WorldError::WindowIdExhausted`] if no ID remains.
    pub fn add_window(&mut self, rect: GridRect) -> Result<WindowId, WorldError> {
        let id = WindowId::from_raw(self.next_window_id);
        self.next_window_id = self
            .next_window_id
            .checked_add(1)
            .ok_or(WorldError::WindowIdExhausted)?;
        self.windows.insert(id, Window::new(id, rect));
        if self.focus.window().is_none() {
            self.focus.set(Some(id));
        }
        Ok(id)
    }

    /// Places a Window beside the anchor captured by an explicit placement request.
    /// Without a request, placement uses the currently focused Window and defaults to
    /// its right side. With no visible Window, it uses the Camera center. A request is
    /// consumed after one successful placement.
    ///
    /// # Errors
    /// Returns an error if IDs are exhausted or the placement exceeds the
    /// supported coordinate model.
    pub fn place_window(&mut self, size: GridSize) -> Result<WindowId, WorldError> {
        let (direction, requested_anchor) = self.next_placement.unwrap_or((Direction::Right, None));
        let visible = self
            .windows
            .values()
            .filter(|window| window.grid_constraint() == GridConstraint::Tiled)
            .filter(|window| self.placement_camera_sees(window.rect()))
            .map(Window::rect)
            .collect::<Vec<_>>();
        let anchor = requested_anchor
            .or_else(|| {
                self.focused().filter(|&id| {
                    self.window(id)
                        .is_some_and(|window| self.placement_camera_sees(window.rect()))
                })
            })
            .and_then(|id| self.window(id))
            .filter(|window| window.grid_constraint() == GridConstraint::Tiled);
        let origin = if visible.is_empty() {
            self.camera_centered_origin(size)?
        } else if let Some(anchor) = anchor {
            self.directional_placement_origin(size, direction, &[anchor.rect()])?
        } else {
            self.directional_placement_origin(size, direction, &visible)?
        };
        let id = self.add_window(self.grid.rect(origin, size)?)?;
        self.next_placement = None;
        Ok(id)
    }

    /// Captures the focused Window and direction for the next successful placement.
    pub fn set_next_placement_direction(&mut self, direction: Direction) {
        self.next_placement = Some((direction, self.focused()));
    }

    /// Removes and returns a window.
    ///
    /// # Errors
    /// Returns [`WorldError::UnknownWindow`] when `id` is absent.
    pub fn remove_window(&mut self, id: WindowId) -> Result<Window, WorldError> {
        let removed = self
            .windows
            .remove(&id)
            .ok_or(WorldError::UnknownWindow(id))?;
        if self.focus.window() == Some(id) {
            self.focus.set(None);
        }
        Ok(removed)
    }

    /// Moves a window to an absolute World position.
    ///
    /// # Errors
    /// Returns an error for an unknown window or overflowing geometry.
    pub fn move_window(&mut self, id: WindowId, origin: GridPoint) -> Result<(), WorldError> {
        self.move_window_continuous(id, origin.into())
    }

    /// Moves a Window to a continuous World position. Tiled Windows reject
    /// non-grid-aligned origins; floating Windows accept any finite origin.
    ///
    /// # Errors
    /// Returns an error for an unknown Window, invalid geometry, or an off-Grid
    /// tiled destination.
    pub fn move_window_continuous(
        &mut self,
        id: WindowId,
        origin: WorldPoint,
    ) -> Result<(), WorldError> {
        let window = self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        if window.grid_constraint() == GridConstraint::Tiled
            && (origin.x.fract() != 0.0 || origin.y.fract() != 0.0)
        {
            return Err(WorldError::OffGrid(id));
        }
        let proposed = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .rect()
            .moved_to(origin)?;
        self.validate_window_geometry(id, proposed)?;
        self.window_mut(id)?.move_to(origin)?;
        Ok(())
    }

    /// Translates a window in grid cells.
    ///
    /// # Errors
    /// Returns an error for an unknown window or overflowing geometry.
    #[allow(clippy::cast_precision_loss)]
    pub fn move_window_by(&mut self, id: WindowId, dx: i64, dy: i64) -> Result<(), WorldError> {
        let origin = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .rect()
            .origin()
            .translated(dx as f64, dy as f64)?;
        self.move_window_continuous(id, origin)
    }

    /// Changes a window's grid size without moving its leading edge.
    ///
    /// When a tiled Window's trailing edge moves, tiled Windows touching that
    /// edge move by the same amount. The relationship is derived from current
    /// Grid geometry and propagates through touching chains. Floating Windows
    /// are never followers. The entire operation is validated before mutation.
    ///
    /// # Errors
    /// Returns an error for an unknown window or overflowing geometry.
    pub fn resize_window(&mut self, id: WindowId, size: GridSize) -> Result<(), WorldError> {
        let window = self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        let old_rect = window.rect();
        if window.grid_constraint() == GridConstraint::Floating
            || (old_rect.width() != size.width() && old_rect.height() != size.height())
        {
            let proposed = old_rect.resized(size)?;
            self.validate_window_geometry(id, proposed)?;
            self.window_mut(id)?.resize(size)?;
            return Ok(());
        }

        if old_rect.width() != size.width() {
            let delta = i64::try_from(size.width()).map_err(|_| GeometryError::Overflow)?
                - i64::try_from(old_rect.width()).map_err(|_| GeometryError::Overflow)?;
            self.resize_tiled_with_followers(id, size, delta, true)
        } else if old_rect.height() != size.height() {
            let delta = i64::try_from(size.height()).map_err(|_| GeometryError::Overflow)?
                - i64::try_from(old_rect.height()).map_err(|_| GeometryError::Overflow)?;
            self.resize_tiled_with_followers(id, size, delta, false)
        } else {
            self.validate_window_geometry(id, old_rect)
        }
    }

    /// Atomically changes all edges of a Window rectangle.
    ///
    /// Tiled Windows touching any changed edge follow that edge using a derived
    /// adjacency chain in the corresponding direction.
    /// # Errors
    /// Returns an error for an unknown Window, invalid geometry, or tiled overlap.
    pub fn resize_window_rect(&mut self, id: WindowId, rect: GridRect) -> Result<(), WorldError> {
        let rect: WorldRect = rect.into();
        self.resize_window_continuous_rect(id, rect)
    }

    /// # Errors
    /// Returns an error for an unknown Window, invalid geometry, or tiled overlap.
    pub fn resize_window_continuous_rect(
        &mut self,
        id: WindowId,
        rect: WorldRect,
    ) -> Result<(), WorldError> {
        let window = self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        let old = window.rect();
        if window.presentation() != Presentation::Normal {
            return Err(WorldError::PresentedWindow(id));
        }
        if window.grid_constraint() == GridConstraint::Floating {
            self.validate_window_geometry(id, rect)?;
            self.window_mut(id)?.set_rect(rect);
            return Ok(());
        }

        let edge_changes = [
            (Direction::Left, rect.x() - old.x()),
            (Direction::Right, rect.right()? - old.right()?),
            (Direction::Up, rect.y() - old.y()),
            (Direction::Down, rect.bottom()? - old.bottom()?),
        ];
        let mut translations = BTreeMap::<WindowId, (f64, f64)>::new();
        for (direction, delta) in edge_changes {
            if delta == 0.0 {
                continue;
            }
            for follower in self.edge_followers(id, direction)? {
                let translation = translations.entry(follower).or_default();
                match direction {
                    Direction::Left | Direction::Right => translation.0 += delta,
                    Direction::Up | Direction::Down => translation.1 += delta,
                }
            }
        }
        let changed = translations
            .keys()
            .copied()
            .chain(std::iter::once(id))
            .collect::<BTreeSet<_>>();
        let follower_proposal = (|| {
            let mut proposed = self
                .windows
                .iter()
                .map(|(&window_id, window)| (window_id, window.rect()))
                .collect::<BTreeMap<_, _>>();
            proposed.insert(id, rect);
            for &follower in changed.iter().filter(|&&candidate| candidate != id) {
                let current = proposed[&follower];
                let (dx, dy) = translations[&follower];
                proposed.insert(follower, current.translated(dx, dy)?);
            }
            for &window_id in &changed {
                let window = self
                    .window(window_id)
                    .ok_or(WorldError::UnknownWindow(window_id))?;
                if window.presentation() != Presentation::Normal {
                    return Err(WorldError::PresentedWindow(window_id));
                }
                let candidate = proposed[&window_id];
                if self.windows.iter().any(|(other, other_window)| {
                    *other != window_id
                        && other_window.grid_constraint() == GridConstraint::Tiled
                        && candidate.overlaps(proposed[other])
                }) {
                    return Err(WorldError::Occupied(candidate));
                }
            }
            Ok(proposed)
        })();

        let proposed = match follower_proposal {
            Ok(proposed) => proposed,
            Err(WorldError::Occupied(_) | WorldError::Geometry(_)) => {
                self.validate_window_geometry(id, rect)?;
                self.window_mut(id)?.set_rect(rect);
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        self.window_mut(id)?.set_rect(rect);
        for &follower in translations.keys() {
            self.window_mut(follower)?.set_rect(proposed[&follower]);
        }
        Ok(())
    }

    #[allow(clippy::float_cmp)]
    /// Derives the tiled Windows connected to one edge through the current geometry.
    /// This is a read-only layout query; it does not create a persistent group.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or overflowing geometry.
    pub fn edge_followers(
        &self,
        id: WindowId,
        direction: Direction,
    ) -> Result<BTreeSet<WindowId>, WorldError> {
        let mut found = BTreeSet::new();
        let mut frontier = vec![id];
        while let Some(current_id) = frontier.pop() {
            let current = self
                .window(current_id)
                .ok_or(WorldError::UnknownWindow(current_id))?
                .rect();
            let edge = match direction {
                Direction::Left => current.x(),
                Direction::Right => current.right()?,
                Direction::Up => current.y(),
                Direction::Down => current.bottom()?,
            };
            for candidate in self.windows.values() {
                if candidate.id() == id
                    || found.contains(&candidate.id())
                    || candidate.grid_constraint() != GridConstraint::Tiled
                {
                    continue;
                }
                let other = candidate.rect();
                let overlaps_perpendicular = match direction {
                    Direction::Left | Direction::Right => {
                        current.y() < other.bottom()? && other.y() < current.bottom()?
                    }
                    Direction::Up | Direction::Down => {
                        current.x() < other.right()? && other.x() < current.right()?
                    }
                };
                let touches = overlaps_perpendicular
                    && match direction {
                        Direction::Left => other.right()? == edge,
                        Direction::Right => other.x() == edge,
                        Direction::Up => other.bottom()? == edge,
                        Direction::Down => other.y() == edge,
                    };
                if touches {
                    found.insert(candidate.id());
                    frontier.push(candidate.id());
                }
            }
        }
        Ok(found)
    }

    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    fn resize_tiled_with_followers(
        &mut self,
        id: WindowId,
        size: GridSize,
        delta: i64,
        horizontal: bool,
    ) -> Result<(), WorldError> {
        let resized = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .rect()
            .resized(size)?;
        let mut changed = BTreeSet::from([id]);
        let mut frontier = vec![id];
        while let Some(current_id) = frontier.pop() {
            let current = self
                .window(current_id)
                .ok_or(WorldError::UnknownWindow(current_id))?
                .rect();
            let edge = if horizontal {
                current.right()?
            } else {
                current.bottom()?
            };
            for candidate in self.windows.values() {
                if changed.contains(&candidate.id())
                    || candidate.grid_constraint() != GridConstraint::Tiled
                {
                    continue;
                }
                let candidate_rect = candidate.rect();
                let touches = if horizontal {
                    candidate_rect.x() == edge
                        && current.y() < candidate_rect.bottom()?
                        && candidate_rect.y() < current.bottom()?
                } else {
                    candidate_rect.y() == edge
                        && current.x() < candidate_rect.right()?
                        && candidate_rect.x() < current.right()?
                };
                if touches {
                    changed.insert(candidate.id());
                    frontier.push(candidate.id());
                }
            }
        }

        let mut proposed = self
            .windows
            .iter()
            .map(|(&window_id, window)| (window_id, window.rect()))
            .collect::<BTreeMap<_, _>>();
        proposed.insert(id, resized);
        for &follower in changed.iter().filter(|&&window_id| window_id != id) {
            let rect = proposed[&follower];
            proposed.insert(
                follower,
                rect.translated(
                    if horizontal { delta as f64 } else { 0.0 },
                    if horizontal { 0.0 } else { delta as f64 },
                )?,
            );
        }

        for &window_id in &changed {
            let window = self
                .window(window_id)
                .ok_or(WorldError::UnknownWindow(window_id))?;
            if window.presentation() != Presentation::Normal {
                return Err(WorldError::PresentedWindow(window_id));
            }
            let candidate = proposed[&window_id];
            if self.windows.iter().any(|(other_id, other_window)| {
                *other_id != window_id
                    && other_window.grid_constraint() == GridConstraint::Tiled
                    && candidate.overlaps(proposed[other_id])
            }) {
                return Err(WorldError::Occupied(candidate));
            }
        }

        self.window_mut(id)?.resize(size)?;
        for &follower in changed.iter().filter(|&&window_id| window_id != id) {
            self.window_mut(follower)?
                .move_to(proposed[&follower].origin())?;
        }
        Ok(())
    }

    /// Toggles the window's Grid occupancy constraint without changing its World rect.
    ///
    /// # Errors
    /// Returns [`WorldError::UnknownWindow`] when `id` is absent.
    pub fn toggle_floating(&mut self, id: WindowId) -> Result<GridConstraint, WorldError> {
        let window = self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        let floating = window.grid_constraint() != GridConstraint::Floating;
        if floating {
            self.set_runtime_window_property(id, WindowProperty::Floating(true))?;
        } else {
            let mut candidate = window.clone();
            candidate.set_runtime_property(WindowProperty::Floating(false));
            let origin = window.rect().origin();
            let snapped = WorldPoint::new(origin.x.round(), origin.y.round())?;
            candidate.set_rect(window.rect().moved_to(snapped)?);
            self.validate_window_constraint(id, &candidate)?;
            let window = self.window_mut(id)?;
            window.set_rect(candidate.rect());
            window.set_runtime_property(WindowProperty::Floating(false));
        }
        Ok(self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .grid_constraint())
    }

    /// Sets a property supplied by matched declarative configuration.
    ///
    /// # Errors
    /// Returns an error for an unknown Window, invalid value, or tiled overlap.
    pub fn set_config_window_property(
        &mut self,
        id: WindowId,
        property: WindowProperty,
    ) -> Result<(), WorldError> {
        self.validate_property_value(id, property)?;
        let mut candidate = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .clone();
        candidate.set_config_property(property);
        self.validate_window_constraint(id, &candidate)?;
        self.window_mut(id)?.set_config_property(property);
        Ok(())
    }

    /// Clears all matched configuration properties without touching runtime overrides.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or if the resulting tiled state overlaps.
    pub fn clear_config_window_properties(&mut self, id: WindowId) -> Result<(), WorldError> {
        let mut candidate = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .clone();
        candidate.clear_config_properties();
        self.validate_window_constraint(id, &candidate)?;
        self.window_mut(id)?.clear_config_properties();
        Ok(())
    }

    /// Atomically replaces properties supplied by matched declarative rules.
    ///
    /// # Errors
    /// Returns an error for an unknown Window, invalid value, or tiled overlap.
    pub fn replace_config_window_properties(
        &mut self,
        id: WindowId,
        properties: &[WindowProperty],
    ) -> Result<(), WorldError> {
        let mut candidate = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .clone();
        candidate.clear_config_properties();
        for &property in properties {
            self.validate_property_value(id, property)?;
            candidate.set_config_property(property);
        }
        self.validate_window_constraint(id, &candidate)?;
        let window = self.window_mut(id)?;
        window.clear_config_properties();
        for &property in properties {
            window.set_config_property(property);
        }
        Ok(())
    }

    /// Sets a per-Window in-memory runtime property override.
    ///
    /// # Errors
    /// Returns an error for an unknown Window, invalid value, or tiled overlap.
    pub fn set_runtime_window_property(
        &mut self,
        id: WindowId,
        property: WindowProperty,
    ) -> Result<(), WorldError> {
        self.validate_property_value(id, property)?;
        let mut candidate = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .clone();
        candidate.set_runtime_property(property);
        self.validate_window_constraint(id, &candidate)?;
        self.window_mut(id)?.set_runtime_property(property);
        Ok(())
    }

    /// Clears a per-Window runtime override, revealing its Config or Default value.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or if the resulting tiled state overlaps.
    pub fn clear_runtime_window_property(
        &mut self,
        id: WindowId,
        kind: WindowPropertyKind,
    ) -> Result<(), WorldError> {
        let mut candidate = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .clone();
        candidate.clear_runtime_property(kind);
        self.validate_window_constraint(id, &candidate)?;
        self.window_mut(id)?.clear_runtime_property(kind);
        Ok(())
    }

    /// Toggles fullscreen presentation using the current Camera viewport.
    ///
    /// # Errors
    /// Returns an error for an unknown window or invalid Camera viewport.
    pub fn toggle_fullscreen(&mut self, id: WindowId) -> Result<Presentation, WorldError> {
        self.toggle_presentation(id, Presentation::Fullscreen)
    }

    /// Toggles maximized presentation using the current Camera viewport.
    ///
    /// # Errors
    /// Returns an error for an unknown window or invalid Camera viewport.
    pub fn toggle_maximized(&mut self, id: WindowId) -> Result<Presentation, WorldError> {
        self.toggle_presentation(id, Presentation::Maximized)
    }

    /// Focuses a window without moving the camera.
    ///
    /// # Errors
    /// Returns [`WorldError::UnknownWindow`] when `id` is absent.
    pub fn focus_window(&mut self, id: WindowId) -> Result<(), WorldError> {
        if !self.windows.contains_key(&id) {
            return Err(WorldError::UnknownWindow(id));
        }
        self.focus.set(Some(id));
        Ok(())
    }

    /// Toggles a Window between the configured initial width and half of that width.
    /// Both targets use the configured initial height.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or an invalid tiled resize.
    pub fn toggle_window_size(
        &mut self,
        id: WindowId,
        initial_size: GridSize,
    ) -> Result<(), WorldError> {
        let current_width = self
            .window(id)
            .ok_or(WorldError::UnknownWindow(id))?
            .rect()
            .width();
        let initial_width = initial_size.width();
        let half_width = initial_width / 2 + initial_width % 2;
        let target_width = if current_width == initial_width || current_width < half_width {
            half_width
        } else {
            initial_width
        };
        let target = GridSize::new(target_width, initial_size.height())?;
        self.resize_window(id, target)
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Option<WindowId> {
        let next = self.directional_neighbor(direction).or_else(|| {
            self.focused()
                .is_none()
                .then(|| self.focus_entry_candidate())
                .flatten()
        })?;
        self.focus.set(Some(next));
        Some(next)
    }

    #[allow(clippy::cast_precision_loss)]
    fn focus_entry_candidate(&self) -> Option<WindowId> {
        let viewport = self.camera.viewport_size();
        let camera_x = self.camera.position().x + viewport.width() as f64 / 2.0;
        let camera_y = self.camera.position().y + viewport.height() as f64 / 2.0;

        self.windows
            .values()
            .map(|window| {
                let (window_x, window_y) = window.rect().center();
                let dx = window_x - camera_x;
                let dy = window_y - camera_y;
                (window.id(), dx.mul_add(dx, dy * dy))
            })
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.0.cmp(&right.0))
            })
            .map(|(id, _)| id)
    }

    #[must_use]
    pub fn directional_neighbor(&self, direction: Direction) -> Option<WindowId> {
        let focused = self.focus.window().and_then(|id| self.windows.get(&id))?;
        let (focus_x, focus_y) = focused.rect().center();

        self.windows
            .values()
            .filter(|candidate| candidate.id() != focused.id())
            .filter_map(|candidate| {
                let (candidate_x, candidate_y) = candidate.rect().center();
                let dx = candidate_x - focus_x;
                let dy = candidate_y - focus_y;
                let (forward, perpendicular) = match direction {
                    Direction::Left => (-dx, dy.abs()),
                    Direction::Right => (dx, dy.abs()),
                    Direction::Up => (-dy, dx.abs()),
                    Direction::Down => (dy, dx.abs()),
                };
                (forward > 0.0).then(|| {
                    let distance_squared = dx.mul_add(dx, dy * dy);
                    (candidate.id(), distance_squared, perpendicular, forward)
                })
            })
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.2.total_cmp(&right.2))
                    .then_with(|| left.3.total_cmp(&right.3))
                    .then_with(|| left.0.cmp(&right.0))
            })
            .map(|(id, _, _, _)| id)
    }

    /// Moves the camera by one viewport without changing window geometry.
    ///
    /// # Errors
    /// Returns an error if the camera would exceed the coordinate model.
    pub fn camera_step(&mut self, direction: Direction) -> Result<(), WorldError> {
        self.camera.step(direction)?;
        Ok(())
    }

    /// Moves the Camera by one World grid cell.
    ///
    /// # Errors
    /// Returns an error if the camera would exceed the coordinate model.
    pub fn camera_nudge(&mut self, direction: Direction) -> Result<(), WorldError> {
        let position = self.camera.position();
        let (dx, dy) = match direction {
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
        };
        self.camera
            .move_to(CameraPosition::new(position.x + dx, position.y + dy)?)?;
        Ok(())
    }

    /// Moves the Camera by a continuous World-coordinate delta.
    ///
    /// # Errors
    /// Returns an error if either delta or the resulting position is non-finite.
    pub fn camera_pan(&mut self, delta_x: f64, delta_y: f64) -> Result<(), WorldError> {
        let position = self.camera.position();
        self.camera.move_to(CameraPosition::new(
            position.x + delta_x,
            position.y + delta_y,
        )?)?;
        Ok(())
    }

    /// Changes the Camera viewport grid dimensions.
    ///
    /// # Errors
    /// Returns an error if the viewport exceeds the coordinate model.
    pub fn resize_camera_viewport(&mut self, size: GridSize) -> Result<(), WorldError> {
        self.camera.resize_viewport(size)?;
        Ok(())
    }

    /// Moves the camera to the natural viewport stop containing the window's center.
    ///
    /// # Errors
    /// Returns an error for an unknown window or overflowing camera position.
    #[allow(clippy::cast_precision_loss)]
    pub fn camera_to_window(&mut self, id: WindowId) -> Result<(), WorldError> {
        let rect = self.window(id).ok_or(WorldError::UnknownWindow(id))?.rect();
        let (center_x, center_y) = rect.center();
        let viewport = self.camera.viewport_size();
        let width = viewport.width() as f64;
        let height = viewport.height() as f64;
        let x = (center_x / width).floor() * width;
        let y = (center_y / height).floor() * height;
        let position = CameraPosition::new(x, y)?;
        self.camera.move_to(position)?;
        Ok(())
    }

    /// Centers the Camera on a Window without changing the Window's World rectangle.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or overflowing geometry.
    pub fn camera_center_window(&mut self, id: WindowId) -> Result<(), WorldError> {
        let rect = self.window(id).ok_or(WorldError::UnknownWindow(id))?.rect();
        let (center_x, center_y) = rect.center();
        let viewport = self.camera.viewport_size();
        #[allow(clippy::cast_precision_loss)]
        let position = CameraPosition::new(
            center_x - viewport.width() as f64 / 2.0,
            center_y - viewport.height() as f64 / 2.0,
        )?;
        self.camera.move_to(position)?;
        Ok(())
    }

    /// Reveals a Window while preserving an already useful Camera composition.
    ///
    /// A fully visible Window leaves the Camera unchanged. A partially visible Window
    /// moves it by only the amount needed for containment. A completely invisible
    /// Window is centered. Visibility is evaluated at the current Camera zoom. A Window
    /// larger than that visible extent is centered without changing zoom.
    ///
    /// # Errors
    /// Returns an error for an unknown Window or overflowing geometry.
    #[allow(clippy::cast_precision_loss)]
    pub fn camera_follow_window(&mut self, id: WindowId) -> Result<(), WorldError> {
        let rect = self.window(id).ok_or(WorldError::UnknownWindow(id))?.rect();
        let position = self.camera.position();
        let viewport = self.camera.viewport_size();
        let zoom = self.camera.zoom();
        let visible_width = viewport.width() as f64 / zoom;
        let visible_height = viewport.height() as f64 / zoom;
        let extra_x = (visible_width - viewport.width() as f64) / 2.0;
        let extra_y = (visible_height - viewport.height() as f64) / 2.0;
        let visible_left = position.x - extra_x;
        let visible_top = position.y - extra_y;
        let visible_right = visible_left + visible_width;
        let visible_bottom = visible_top + visible_height;
        let rect_right = rect.right()?;
        let rect_bottom = rect.bottom()?;

        let is_completely_outside = rect_right <= visible_left
            || rect.x() >= visible_right
            || rect_bottom <= visible_top
            || rect.y() >= visible_bottom;
        if is_completely_outside
            || rect.width() as f64 > visible_width
            || rect.height() as f64 > visible_height
        {
            return self.camera_center_window(id);
        }

        let x = if rect.x() < visible_left {
            rect.x() + extra_x
        } else if rect_right > visible_right {
            rect_right - visible_width + extra_x
        } else {
            position.x
        };
        let y = if rect.y() < visible_top {
            rect.y() + extra_y
        } else if rect_bottom > visible_bottom {
            rect_bottom - visible_height + extra_y
        } else {
            position.y
        };
        self.camera.move_to(CameraPosition::new(x, y)?)?;
        Ok(())
    }

    /// Changes Camera magnification without changing any Window geometry.
    ///
    /// # Errors
    /// Returns an error for a non-finite or non-positive zoom.
    pub fn camera_zoom(&mut self, zoom: f64) -> Result<(), WorldError> {
        self.camera.set_zoom(zoom)?;
        Ok(())
    }

    /// Applies a shared Action to logical Core state.
    ///
    /// # Errors
    /// Returns an error when an action references an unknown window or produces
    /// geometry outside the coordinate model.
    pub fn apply(&mut self, action: Action) -> Result<ActionOutcome, WorldError> {
        self.ensure_camera_action_available(action)?;
        match action {
            Action::Focus(direction) => {
                Ok(ActionOutcome::FocusChanged(self.focus_direction(direction)))
            }
            Action::FocusWindow(id) => {
                self.focus_window(id)?;
                Ok(ActionOutcome::FocusChanged(Some(id)))
            }
            Action::MoveWindow { id, origin } => {
                self.move_window(id, origin)?;
                Ok(ActionOutcome::Applied)
            }
            Action::MoveWindowContinuous { id, origin } => {
                self.move_window_continuous(id, origin)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ResizeWindow { id, size } => {
                self.resize_window(id, size)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ResizeWindowRect { id, rect } => {
                self.resize_window_rect(id, rect)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ResizeWindowContinuousRect { id, rect } => {
                self.resize_window_continuous_rect(id, rect)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ToggleWindowSize { id, initial_size } => {
                self.toggle_window_size(id, initial_size)?;
                Ok(ActionOutcome::Applied)
            }
            Action::SetNextPlacement(direction) => {
                self.set_next_placement_direction(direction);
                Ok(ActionOutcome::Applied)
            }
            Action::ActivateOutput(id) => {
                self.activate_output(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CycleOutput => {
                self.cycle_output();
                Ok(ActionOutcome::Applied)
            }
            Action::CameraStep(direction) => {
                self.camera_step(direction)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraNudge(direction) => {
                self.camera_nudge(direction)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraPan { delta_x, delta_y } => {
                self.camera_pan(delta_x, delta_y)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraTo(id) => {
                self.camera_to_window(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraCenter(id) => {
                self.camera_center_window(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraFollow(id) => {
                self.camera_follow_window(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CameraZoom(zoom) => {
                self.camera_zoom(zoom)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ToggleFloating(id) => {
                self.toggle_floating(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::SetWindowProperty { id, property } => {
                self.set_runtime_window_property(id, property)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ClearWindowProperty { id, kind } => {
                self.clear_runtime_window_property(id, kind)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ToggleFullscreen(id) => {
                self.toggle_fullscreen(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::ToggleMaximized(id) => {
                self.toggle_maximized(id)?;
                Ok(ActionOutcome::Applied)
            }
            Action::CloseWindow(id) => {
                if !self.windows.contains_key(&id) {
                    return Err(WorldError::UnknownWindow(id));
                }
                Ok(ActionOutcome::CloseRequested(id))
            }
        }
    }

    fn ensure_camera_action_available(&self, action: Action) -> Result<(), WorldError> {
        let moves_camera = matches!(
            action,
            Action::CameraStep(_)
                | Action::CameraNudge(_)
                | Action::CameraPan { .. }
                | Action::CameraTo(_)
                | Action::CameraCenter(_)
                | Action::CameraFollow(_)
                | Action::CameraZoom(_)
        );
        if !moves_camera {
            return Ok(());
        }
        self.windows
            .values()
            .find(|window| window.presentation() == Presentation::Fullscreen)
            .map_or(Ok(()), |window| {
                Err(WorldError::CameraLockedByFullscreen(window.id()))
            })
    }

    /// Returns windows intersecting the camera and each visible clip rectangle.
    ///
    /// # Errors
    /// Returns an error if the camera viewport exceeds the coordinate model.
    pub fn visible_windows(
        &self,
    ) -> Result<impl Iterator<Item = (&Window, WorldRect)>, WorldError> {
        let viewport: WorldRect = self.camera.viewport()?.into();
        Ok(self.windows.values().filter_map(move |window| {
            window
                .rect()
                .intersection(viewport)
                .map(|clip| (window, clip))
        }))
    }

    fn window_mut(&mut self, id: WindowId) -> Result<&mut Window, WorldError> {
        self.windows
            .get_mut(&id)
            .ok_or(WorldError::UnknownWindow(id))
    }

    #[allow(clippy::cast_precision_loss)]
    fn placement_camera_sees(&self, rect: WorldRect) -> bool {
        let camera = self.camera;
        let viewport = camera.viewport_size();
        let left = camera.position().x;
        let top = camera.position().y;
        let right = left + viewport.width() as f64;
        let bottom = top + viewport.height() as f64;
        rect.x() < right
            && rect.right().is_ok_and(|window_right| window_right > left)
            && rect.y() < bottom
            && rect.bottom().is_ok_and(|window_bottom| window_bottom > top)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn camera_centered_origin(&self, size: GridSize) -> Result<GridPoint, WorldError> {
        let viewport = self.camera.viewport_size();
        let center_x = self.camera.position().x + viewport.width() as f64 / 2.0;
        let center_y = self.camera.position().y + viewport.height() as f64 / 2.0;
        let x = (center_x - size.width() as f64 / 2.0).floor();
        let y = (center_y - size.height() as f64 / 2.0).floor();
        if x < i64::MIN as f64 || x > i64::MAX as f64 || y < i64::MIN as f64 || y > i64::MAX as f64
        {
            return Err(GeometryError::Overflow.into());
        }
        Ok(GridPoint::new(x as i64, y as i64))
    }

    fn directional_placement_origin(
        &self,
        size: GridSize,
        direction: Direction,
        visible: &[WorldRect],
    ) -> Result<GridPoint, WorldError> {
        let anchor = match direction {
            Direction::Left => visible.iter().min_by(|a, b| a.x().total_cmp(&b.x())),
            Direction::Right => visible.iter().max_by(|a, b| {
                a.right()
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&b.right().unwrap_or(f64::INFINITY))
            }),
            Direction::Up => visible.iter().min_by(|a, b| a.y().total_cmp(&b.y())),
            Direction::Down => visible.iter().max_by(|a, b| {
                a.bottom()
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&b.bottom().unwrap_or(f64::INFINITY))
            }),
        }
        .copied()
        .ok_or(GeometryError::Overflow)?;
        let width = i64::try_from(size.width()).map_err(|_| GeometryError::Overflow)?;
        let height = i64::try_from(size.height()).map_err(|_| GeometryError::Overflow)?;
        #[allow(clippy::cast_possible_truncation)]
        let origin = match direction {
            Direction::Left => GridPoint::new(
                (anchor.x().floor() as i64)
                    .checked_sub(width)
                    .ok_or(GeometryError::Overflow)?,
                anchor.y().floor() as i64,
            ),
            Direction::Right => {
                GridPoint::new(anchor.right()?.ceil() as i64, anchor.y().floor() as i64)
            }
            Direction::Up => GridPoint::new(
                anchor.x().floor() as i64,
                (anchor.y().floor() as i64)
                    .checked_sub(height)
                    .ok_or(GeometryError::Overflow)?,
            ),
            Direction::Down => {
                GridPoint::new(anchor.x().floor() as i64, anchor.bottom()?.ceil() as i64)
            }
        };
        self.advance_placement_past_occupied(origin, size, direction)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn advance_placement_past_occupied(
        &self,
        mut origin: GridPoint,
        size: GridSize,
        direction: Direction,
    ) -> Result<GridPoint, WorldError> {
        loop {
            let candidate: WorldRect = self.grid.rect(origin, size)?.into();
            let blockers = self
                .windows
                .values()
                .filter(|window| window.grid_constraint() == GridConstraint::Tiled)
                .map(Window::rect)
                .filter(|rect| rect.overlaps(candidate))
                .collect::<Vec<_>>();
            if blockers.is_empty() {
                return Ok(origin);
            }
            let width = i64::try_from(size.width()).map_err(|_| GeometryError::Overflow)?;
            let height = i64::try_from(size.height()).map_err(|_| GeometryError::Overflow)?;
            origin = match direction {
                Direction::Left => GridPoint::new(
                    blockers
                        .iter()
                        .map(|rect| rect.x())
                        .min_by(f64::total_cmp)
                        .ok_or(GeometryError::Overflow)?
                        .floor() as i64
                        - width,
                    origin.y,
                ),
                Direction::Right => GridPoint::new(
                    blockers
                        .iter()
                        .map(|rect| rect.right())
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .max_by(f64::total_cmp)
                        .ok_or(GeometryError::Overflow)?
                        .ceil() as i64,
                    origin.y,
                ),
                Direction::Up => GridPoint::new(
                    origin.x,
                    blockers
                        .iter()
                        .map(|rect| rect.y())
                        .min_by(f64::total_cmp)
                        .ok_or(GeometryError::Overflow)?
                        .floor() as i64
                        - height,
                ),
                Direction::Down => GridPoint::new(
                    origin.x,
                    blockers
                        .iter()
                        .map(|rect| rect.bottom())
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .max_by(f64::total_cmp)
                        .ok_or(GeometryError::Overflow)?
                        .ceil() as i64,
                ),
            };
        }
    }

    fn validate_window_geometry(
        &self,
        id: WindowId,
        proposed: WorldRect,
    ) -> Result<(), WorldError> {
        let window = self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        if window.presentation() != Presentation::Normal {
            return Err(WorldError::PresentedWindow(id));
        }
        if window.grid_constraint() == GridConstraint::Tiled
            && self.windows.values().any(|other| {
                other.id() != id
                    && other.grid_constraint() == GridConstraint::Tiled
                    && other.rect().overlaps(proposed)
            })
        {
            return Err(WorldError::Occupied(proposed));
        }
        Ok(())
    }

    fn validate_property_value(
        &self,
        id: WindowId,
        property: WindowProperty,
    ) -> Result<(), WorldError> {
        self.window(id).ok_or(WorldError::UnknownWindow(id))?;
        match property {
            WindowProperty::Opacity(value)
                if !value.is_finite() || !(0.0..=1.0).contains(&value) =>
            {
                Err(WorldError::InvalidOpacity)
            }
            _ => Ok(()),
        }
    }

    fn validate_window_constraint(
        &self,
        id: WindowId,
        candidate: &Window,
    ) -> Result<(), WorldError> {
        if candidate.grid_constraint() == GridConstraint::Tiled {
            let origin = candidate.rect().origin();
            if origin.x.fract() != 0.0 || origin.y.fract() != 0.0 {
                return Err(WorldError::OffGrid(id));
            }
            if self.windows.values().any(|other| {
                other.id() != id
                    && other.grid_constraint() == GridConstraint::Tiled
                    && other.rect().overlaps(candidate.rect())
            }) {
                return Err(WorldError::Occupied(candidate.rect()));
            }
        }
        Ok(())
    }

    fn toggle_presentation(
        &mut self,
        id: WindowId,
        presentation: Presentation,
    ) -> Result<Presentation, WorldError> {
        let viewport = self.camera.viewport()?;
        let window = self.window_mut(id)?;
        window.set_presentation(presentation, viewport);
        Ok(window.presentation())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldError {
    UnknownWindow(WindowId),
    WindowIdExhausted,
    Occupied(WorldRect),
    OffGrid(WindowId),
    PresentedWindow(WindowId),
    CameraLockedByFullscreen(WindowId),
    InvalidOpacity,
    Zoom(ZoomError),
    CameraPosition(CameraPositionError),
    Geometry(GeometryError),
    Output(OutputError),
}

impl fmt::Display for WorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWindow(id) => write!(formatter, "unknown window {}", id.get()),
            Self::WindowIdExhausted => formatter.write_str("window ID space exhausted"),
            Self::Occupied(rect) => write!(formatter, "tiled grid region is occupied: {rect:?}"),
            Self::OffGrid(id) => write!(
                formatter,
                "tiled window {} must remain aligned to the Grid",
                id.get()
            ),
            Self::PresentedWindow(id) => write!(
                formatter,
                "window {} must leave fullscreen/maximized state before geometry changes",
                id.get()
            ),
            Self::CameraLockedByFullscreen(id) => write!(
                formatter,
                "camera navigation is locked while window {} is fullscreen",
                id.get()
            ),
            Self::InvalidOpacity => {
                formatter.write_str("opacity must be finite and between 0 and 1")
            }
            Self::Zoom(error) => error.fmt(formatter),
            Self::CameraPosition(error) => error.fmt(formatter),
            Self::Geometry(error) => error.fmt(formatter),
            Self::Output(error) => error.fmt(formatter),
        }
    }
}

impl Error for WorldError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Geometry(error) => Some(error),
            Self::Zoom(error) => Some(error),
            Self::CameraPosition(error) => Some(error),
            Self::Output(error) => Some(error),
            Self::UnknownWindow(_)
            | Self::WindowIdExhausted
            | Self::Occupied(_)
            | Self::PresentedWindow(_)
            | Self::CameraLockedByFullscreen(_)
            | Self::InvalidOpacity
            | Self::OffGrid(_) => None,
        }
    }
}

impl From<GeometryError> for WorldError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

impl From<OutputError> for WorldError {
    fn from(error: OutputError) -> Self {
        Self::Output(error)
    }
}

impl From<ZoomError> for WorldError {
    fn from(error: ZoomError) -> Self {
        Self::Zoom(error)
    }
}

impl From<CameraPositionError> for WorldError {
    fn from(error: CameraPositionError) -> Self {
        Self::CameraPosition(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new(Camera::new(
            GridPoint::new(0, 0),
            GridSize::new(4, 4).unwrap(),
        ))
    }

    fn assert_opacity(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < f32::EPSILON);
    }

    fn assert_camera_position(world: &World, x: f64, y: f64) {
        assert_eq!(
            world.camera().position(),
            CameraPosition::new(x, y).unwrap()
        );
    }

    fn rect(x: i64, y: i64, width: u64, height: u64) -> GridRect {
        GridRect::new(x, y, width, height).unwrap()
    }

    #[test]
    fn stores_windows_at_negative_and_off_camera_coordinates() {
        let mut world = world();
        let negative = world.add_window(rect(-8, -3, 2, 2)).unwrap();
        let off_camera = world.add_window(rect(20, 10, 2, 2)).unwrap();
        assert_eq!(world.window(negative).unwrap().rect(), rect(-8, -3, 2, 2));
        assert!(world.window(off_camera).is_some());
        assert_eq!(world.visible_windows().unwrap().count(), 0);
    }

    #[test]
    fn moves_and_resizes_in_grid_cells() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world.move_window_by(id, -3, 5).unwrap();
        world
            .resize_window(id, GridSize::new(3, 2).unwrap())
            .unwrap();
        assert_eq!(world.window(id).unwrap().rect(), rect(-3, 5, 3, 2));
    }

    #[test]
    fn tiled_resize_pushes_and_pulls_a_touching_chain() {
        let mut world = world();
        let first = world.add_window(rect(-4, 0, 2, 2)).unwrap();
        let second = world.add_window(rect(-2, 0, 2, 2)).unwrap();
        let third = world.add_window(rect(0, 0, 2, 2)).unwrap();

        world
            .apply(Action::ResizeWindow {
                id: first,
                size: GridSize::new(3, 2).unwrap(),
            })
            .unwrap();
        assert_eq!(world.window(first).unwrap().rect(), rect(-4, 0, 3, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(-1, 0, 2, 2));
        assert_eq!(world.window(third).unwrap().rect(), rect(1, 0, 2, 2));

        world
            .apply(Action::ResizeWindow {
                id: first,
                size: GridSize::new(2, 2).unwrap(),
            })
            .unwrap();
        assert_eq!(world.window(first).unwrap().rect(), rect(-4, 0, 2, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(-2, 0, 2, 2));
        assert_eq!(world.window(third).unwrap().rect(), rect(0, 0, 2, 2));
    }

    #[test]
    fn tiled_resize_moves_every_touching_branch() {
        let mut world = world();
        let source = world.add_window(rect(0, 0, 2, 4)).unwrap();
        let upper = world.add_window(rect(2, 0, 2, 2)).unwrap();
        let lower = world.add_window(rect(2, 2, 2, 2)).unwrap();

        world
            .resize_window(source, GridSize::new(3, 4).unwrap())
            .unwrap();

        assert_eq!(world.window(upper).unwrap().rect(), rect(3, 0, 2, 2));
        assert_eq!(world.window(lower).unwrap().rect(), rect(3, 2, 2, 2));
    }

    #[test]
    fn rectangle_resize_is_atomic_and_moves_trailing_edge_followers_on_both_axes() {
        let mut world = world();
        let source = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let right = world.add_window(rect(2, 0, 2, 2)).unwrap();
        let below = world.add_window(rect(0, 2, 2, 2)).unwrap();

        world
            .apply(Action::ResizeWindowRect {
                id: source,
                rect: rect(-1, -1, 4, 4),
            })
            .unwrap();

        assert_eq!(world.window(source).unwrap().rect(), rect(-1, -1, 4, 4));
        assert_eq!(world.window(right).unwrap().rect(), rect(3, 0, 2, 2));
        assert_eq!(world.window(below).unwrap().rect(), rect(0, 3, 2, 2));
    }

    #[test]
    fn leading_edge_resize_moves_touching_followers() {
        let mut world = world();
        let blocker = world.add_window(rect(-2, 0, 2, 2)).unwrap();
        let source = world.add_window(rect(0, 0, 2, 2)).unwrap();

        world.resize_window_rect(source, rect(-1, 0, 3, 2)).unwrap();
        assert_eq!(world.window(blocker).unwrap().rect(), rect(-3, 0, 2, 2));
        assert_eq!(world.window(source).unwrap().rect(), rect(-1, 0, 3, 2));
    }

    #[test]
    fn top_edge_resize_moves_touching_chain() {
        let mut world = world();
        let first = world.add_window(rect(0, -4, 2, 2)).unwrap();
        let second = world.add_window(rect(0, -2, 2, 2)).unwrap();
        let source = world.add_window(rect(0, 0, 2, 2)).unwrap();

        world.resize_window_rect(source, rect(0, -1, 2, 3)).unwrap();
        assert_eq!(world.window(first).unwrap().rect(), rect(0, -5, 2, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(0, -3, 2, 2));
        assert_eq!(world.window(source).unwrap().rect(), rect(0, -1, 2, 3));
    }

    #[test]
    fn resize_keeps_followers_in_place_when_only_their_move_conflicts() {
        let mut world = world();
        let source = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let follower = world.add_window(rect(2, -1, 2, 4)).unwrap();
        let blocker = world.add_window(rect(1, 2, 1, 1)).unwrap();

        world.resize_window_rect(source, rect(0, 0, 1, 2)).unwrap();

        assert_eq!(world.window(source).unwrap().rect(), rect(0, 0, 1, 2));
        assert_eq!(world.window(follower).unwrap().rect(), rect(2, -1, 2, 4));
        assert_eq!(world.window(blocker).unwrap().rect(), rect(1, 2, 1, 1));
    }

    #[test]
    fn floating_window_does_not_block_or_follow_a_tiled_resize() {
        let mut world = world();
        let source = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let floating = world.add_window(rect(2, 0, 2, 2)).unwrap();
        world.toggle_floating(floating).unwrap();
        world
            .resize_window(source, GridSize::new(3, 2).unwrap())
            .unwrap();

        assert_eq!(world.window(source).unwrap().rect(), rect(0, 0, 3, 2));
        assert_eq!(world.window(floating).unwrap().rect(), rect(2, 0, 2, 2));
    }

    #[test]
    fn tiled_window_can_move_beneath_a_floating_window() {
        let mut world = world();
        let tiled = world.add_window(rect(-3, 0, 2, 2)).unwrap();
        let floating = world.add_window(rect(1, 0, 2, 2)).unwrap();
        world.toggle_floating(floating).unwrap();

        world.move_window(tiled, GridPoint::new(1, 0)).unwrap();

        assert_eq!(world.window(tiled).unwrap().rect(), rect(1, 0, 2, 2));
        assert_eq!(world.window(floating).unwrap().rect(), rect(1, 0, 2, 2));
    }

    #[test]
    fn floating_overlap_does_not_prevent_returning_to_tiled() {
        let mut world = world();
        let first = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let second = world.add_window(rect(0, 0, 2, 2)).unwrap();
        world.toggle_floating(first).unwrap();
        world.toggle_floating(second).unwrap();

        world.toggle_floating(first).unwrap();

        assert_eq!(
            world.window(first).unwrap().grid_constraint(),
            GridConstraint::Tiled
        );
        assert_eq!(
            world.window(second).unwrap().grid_constraint(),
            GridConstraint::Floating
        );
    }

    #[test]
    fn overlapping_windows_are_preserved() {
        let mut world = world();
        let first = world.add_window(rect(0, 0, 3, 3)).unwrap();
        let second = world.add_window(rect(2, 2, 3, 3)).unwrap();
        assert!(world
            .window(first)
            .unwrap()
            .rect()
            .overlaps(world.window(second).unwrap().rect()));
        assert_eq!(world.windows().len(), 2);
    }

    #[test]
    fn window_crosses_camera_boundary_without_special_state() {
        let mut world = world();
        let id = world.add_window(rect(3, 1, 3, 2)).unwrap();
        let visible = world.visible_windows().unwrap().next().unwrap().1;
        assert_eq!(visible, rect(3, 1, 1, 2));

        let original = world.window(id).unwrap().rect();
        world.camera_step(Direction::Right).unwrap();
        let visible = world.visible_windows().unwrap().next().unwrap().1;
        assert_eq!(visible, rect(4, 1, 2, 2));
        assert_eq!(world.window(id).unwrap().rect(), original);
    }

    #[test]
    fn directional_focus_works_in_all_directions_including_off_camera() {
        let mut world = world();
        let center = world.add_window(rect(0, 0, 1, 1)).unwrap();
        let left = world.add_window(rect(-10, 0, 1, 1)).unwrap();
        let right = world.add_window(rect(10, 0, 1, 1)).unwrap();
        let up = world.add_window(rect(0, -10, 1, 1)).unwrap();
        let down = world.add_window(rect(0, 10, 1, 1)).unwrap();

        for (direction, expected) in [
            (Direction::Left, left),
            (Direction::Right, right),
            (Direction::Up, up),
            (Direction::Down, down),
        ] {
            world.focus_window(center).unwrap();
            assert_eq!(world.focus_direction(direction), Some(expected));
        }
    }

    #[test]
    fn directional_focus_prefers_nearby_window_over_distant_straight_window() {
        let mut world = world();
        let focused = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let nearby = world.add_window(rect(3, 1, 2, 2)).unwrap();
        let distant = world.add_window(rect(12, 0, 2, 2)).unwrap();
        world.focus_window(focused).unwrap();

        assert_eq!(world.directional_neighbor(Direction::Right), Some(nearby));
        assert_ne!(world.directional_neighbor(Direction::Right), Some(distant));
    }

    #[test]
    fn removing_focused_window_leaves_focus_empty() {
        let mut world = world();
        let first = world.add_window(rect(0, 0, 1, 1)).unwrap();
        let second = world.add_window(rect(2, 0, 1, 1)).unwrap();
        world.focus_window(second).unwrap();
        world.remove_window(second).unwrap();
        assert_eq!(world.focused(), None);
        assert!(world.window(first).is_some());
    }

    #[test]
    fn directional_focus_recovers_from_empty_focus_near_the_camera() {
        let mut world = world();
        let near = world.add_window(rect(1, 1, 1, 1)).unwrap();
        let removed = world.add_window(rect(2, 1, 1, 1)).unwrap();
        let far = world.add_window(rect(20, 20, 1, 1)).unwrap();
        world.focus_window(removed).unwrap();
        world.remove_window(removed).unwrap();

        assert_eq!(world.focused(), None);
        assert_eq!(world.focus_direction(Direction::Right), Some(near));
        assert_ne!(world.focused(), Some(far));
    }

    #[test]
    fn placement_uses_free_world_space_without_resizing_existing_windows() {
        let mut world = world();
        let first = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();
        world.camera_center_window(first).unwrap();
        let second = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();
        assert_eq!(world.window(first).unwrap().rect(), rect(1, 1, 2, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(3, 1, 2, 2));
        assert!(!world
            .window(first)
            .unwrap()
            .rect()
            .overlaps(world.window(second).unwrap().rect()));
    }

    #[test]
    fn floating_focus_does_not_shift_new_tiled_window_placement() {
        let mut world = world();
        let tiled = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let floating = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world.toggle_floating(floating).unwrap();
        world
            .move_window_continuous(floating, WorldPoint::new(1.5, 2.75).unwrap())
            .unwrap();
        world.focus_window(floating).unwrap();

        let placed = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(placed).unwrap().rect(), rect(2, 0, 2, 2));
        assert_eq!(world.window(tiled).unwrap().rect(), rect(0, 0, 2, 2));
    }

    #[test]
    fn floating_only_view_places_new_tiled_window_at_camera_center() {
        let mut world = world();
        let floating = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world.toggle_floating(floating).unwrap();
        world
            .move_window_continuous(floating, WorldPoint::new(1.25, 1.25).unwrap())
            .unwrap();
        world.focus_window(floating).unwrap();

        let placed = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(placed).unwrap().rect(), rect(1, 1, 2, 2));
    }

    #[test]
    fn placement_stays_in_one_row_as_camera_recenters() {
        let mut world = world();
        let first = world.place_window(GridSize::new(3, 2).unwrap()).unwrap();
        world.focus_window(first).unwrap();
        world.camera_center_window(first).unwrap();
        let second = world.place_window(GridSize::new(3, 2).unwrap()).unwrap();
        world.focus_window(second).unwrap();
        world.camera_center_window(second).unwrap();
        let third = world.place_window(GridSize::new(2, 3).unwrap()).unwrap();

        assert_eq!(world.window(first).unwrap().rect(), rect(0, 1, 3, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(3, 1, 3, 2));
        assert_eq!(world.window(third).unwrap().rect(), rect(6, 1, 2, 3));
    }

    #[test]
    fn placement_uses_camera_center_when_no_window_is_visible() {
        let mut world = world();
        world.camera_step(Direction::Right).unwrap();

        let id = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(id).unwrap().rect(), rect(5, 1, 2, 2));
    }

    #[test]
    fn requested_placement_direction_is_consumed_once() {
        let mut world = world();
        let first = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();
        world
            .apply(Action::SetNextPlacement(Direction::Left))
            .unwrap();
        let left = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();
        let default_right = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(first).unwrap().rect(), rect(1, 1, 2, 2));
        assert_eq!(world.window(left).unwrap().rect(), rect(-1, 1, 2, 2));
        assert_eq!(
            world.window(default_right).unwrap().rect(),
            rect(3, 1, 2, 2)
        );
    }

    #[test]
    fn requested_placement_uses_the_window_focused_when_requested() {
        let mut world = world();
        let lower = world.add_window(rect(0, 2, 2, 2)).unwrap();
        let upper = world.add_window(rect(0, 0, 2, 2)).unwrap();
        world.focus_window(lower).unwrap();
        world
            .apply(Action::SetNextPlacement(Direction::Right))
            .unwrap();

        // A later focus change must not silently change the placement anchor.
        world.focus_window(upper).unwrap();
        let placed = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(placed).unwrap().rect(), rect(2, 2, 2, 2));
    }

    #[test]
    fn default_placement_uses_the_currently_focused_window() {
        let mut world = world();
        let lower = world.add_window(rect(0, 2, 2, 2)).unwrap();
        world.add_window(rect(0, 0, 2, 2)).unwrap();
        world.focus_window(lower).unwrap();

        let placed = world.place_window(GridSize::new(2, 2).unwrap()).unwrap();

        assert_eq!(world.window(placed).unwrap().rect(), rect(2, 2, 2, 2));
    }

    #[test]
    fn placement_visibility_uses_normal_viewport_during_overview() {
        let mut world = world();
        world.add_window(rect(5, 1, 1, 1)).unwrap();
        world.camera_zoom(0.5).unwrap();

        let id = world.place_window(GridSize::new(1, 1).unwrap()).unwrap();

        assert_eq!(world.window(id).unwrap().rect(), rect(1, 1, 1, 1));
    }

    #[test]
    fn mouse_style_camera_pan_places_at_new_view_despite_old_focus() {
        let mut world = world();
        let old = world.add_window(rect(0, 0, 4, 4)).unwrap();
        world.focus_window(old).unwrap();
        world.camera_pan(12.5, -9.25).unwrap();

        let placed = world.place_window(GridSize::new(4, 4).unwrap()).unwrap();

        assert_eq!(world.window(placed).unwrap().rect(), rect(12, -10, 4, 4));
    }

    #[test]
    fn focus_and_camera_actions_remain_composable() {
        let mut world = world();
        let origin = world.add_window(rect(0, 0, 1, 1)).unwrap();
        let target = world.add_window(rect(-9, 0, 2, 2)).unwrap();
        world.focus_window(origin).unwrap();

        assert_eq!(
            world.apply(Action::Focus(Direction::Left)).unwrap(),
            ActionOutcome::FocusChanged(Some(target))
        );
        assert_camera_position(&world, 0.0, 0.0);
        world.apply(Action::CameraTo(target)).unwrap();
        assert_camera_position(&world, -8.0, 0.0);
    }

    #[test]
    fn camera_follow_moves_only_axes_outside_the_viewport() {
        let mut world = world();
        let id = world.add_window(rect(2, 1, 3, 2)).unwrap();

        world.apply(Action::CameraFollow(id)).unwrap();
        assert_camera_position(&world, 1.0, 0.0);

        world.move_window(id, GridPoint::new(0, -1)).unwrap();
        world.apply(Action::CameraFollow(id)).unwrap();
        assert_camera_position(&world, 0.0, -1.0);

        world.move_window(id, GridPoint::new(0, 0)).unwrap();
        world.apply(Action::CameraFollow(id)).unwrap();
        assert_camera_position(&world, 0.0, -1.0);
    }

    #[test]
    fn camera_follow_centers_a_completely_invisible_window() {
        let mut world = world();
        let id = world.add_window(rect(-8, 6, 2, 2)).unwrap();

        world.apply(Action::CameraFollow(id)).unwrap();

        assert_camera_position(&world, -9.0, 5.0);
    }

    #[test]
    fn resize_to_initial_size_and_camera_follow_compose_for_pointer_reset() {
        let mut world = world();
        let id = world.add_window(rect(12, -8, 2, 2)).unwrap();
        let initial_size = GridSize::new(4, 4).unwrap();

        world
            .apply(Action::ResizeWindow {
                id,
                size: initial_size,
            })
            .unwrap();
        world.apply(Action::FocusWindow(id)).unwrap();
        world.apply(Action::CameraFollow(id)).unwrap();

        assert_eq!(world.window(id).unwrap().rect().size(), initial_size);
        assert_camera_position(&world, 12.0, -8.0);
    }

    #[test]
    fn window_size_toggle_uses_half_and_initial_width_thresholds() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 2, 3)).unwrap();
        let initial_size = GridSize::new(8, 6).unwrap();
        let toggle = Action::ToggleWindowSize { id, initial_size };

        world.apply(toggle).unwrap();
        assert_eq!(
            world.window(id).unwrap().rect().size(),
            GridSize::new(4, 6).unwrap()
        );

        world.apply(toggle).unwrap();
        assert_eq!(world.window(id).unwrap().rect().size(), initial_size);

        world.apply(toggle).unwrap();
        assert_eq!(
            world.window(id).unwrap().rect().size(),
            GridSize::new(4, 6).unwrap()
        );

        world
            .apply(Action::ResizeWindow {
                id,
                size: GridSize::new(6, 2).unwrap(),
            })
            .unwrap();
        world.apply(toggle).unwrap();
        assert_eq!(world.window(id).unwrap().rect().size(), initial_size);
    }

    #[test]
    fn camera_follow_uses_the_current_zoomed_visible_extent() {
        let mut world = world();
        let id = world.add_window(rect(-2, -2, 8, 8)).unwrap();
        world.apply(Action::CameraZoom(0.5)).unwrap();

        world.apply(Action::CameraFollow(id)).unwrap();

        assert_camera_position(&world, 0.0, 0.0);
        assert!((world.camera().zoom() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn camera_follow_centers_a_window_larger_than_the_zoomed_view_without_zooming() {
        let mut world = world();
        let id = world.add_window(rect(-8, 3, 10, 12)).unwrap();
        world.apply(Action::CameraZoom(0.5)).unwrap();

        world.apply(Action::CameraFollow(id)).unwrap();

        assert_camera_position(&world, -5.0, 7.0);
        assert!((world.camera().zoom() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn camera_center_and_nudge_are_composable() {
        let mut world = world();
        let id = world.add_window(rect(8, -3, 3, 2)).unwrap();

        world.apply(Action::CameraCenter(id)).unwrap();
        assert_camera_position(&world, 7.5, -4.0);
        world.apply(Action::CameraNudge(Direction::Right)).unwrap();
        world.apply(Action::CameraNudge(Direction::Down)).unwrap();
        assert_camera_position(&world, 8.5, -3.0);
        assert_eq!(world.window(id).unwrap().rect(), rect(8, -3, 3, 2));
    }

    #[test]
    fn camera_pan_accepts_fractional_and_negative_world_deltas() {
        let mut world = world();
        world
            .apply(Action::CameraPan {
                delta_x: -0.75,
                delta_y: 1.25,
            })
            .unwrap();
        assert_camera_position(&world, -0.75, 1.25);
        assert!(world
            .apply(Action::CameraPan {
                delta_x: f64::NAN,
                delta_y: 0.0,
            })
            .is_err());
        assert_camera_position(&world, -0.75, 1.25);
    }

    #[test]
    fn camera_zoom_preserves_world_layout() {
        let mut world = world();
        let first = world.add_window(rect(-3, 1, 2, 2)).unwrap();
        let second = world.add_window(rect(5, -2, 3, 1)).unwrap();
        let before = world
            .windows()
            .map(|window| (window.id(), window.rect()))
            .collect::<Vec<_>>();

        world.apply(Action::CameraZoom(0.35)).unwrap();
        assert!((world.camera().zoom() - 0.35).abs() < f64::EPSILON);
        let after = world
            .windows()
            .map(|window| (window.id(), window.rect()))
            .collect::<Vec<_>>();
        assert_eq!(after, before);
        assert_eq!(world.window(first).unwrap().rect(), rect(-3, 1, 2, 2));
        assert_eq!(world.window(second).unwrap().rect(), rect(5, -2, 3, 1));
    }

    #[test]
    fn close_action_does_not_destroy_adapter_owned_client() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 1, 1)).unwrap();
        assert_eq!(
            world.apply(Action::CloseWindow(id)).unwrap(),
            ActionOutcome::CloseRequested(id)
        );
        assert!(world.window(id).is_some());
    }

    #[test]
    fn tiled_geometry_rejects_overlap_while_floating_allows_it() {
        let mut world = world();
        let first = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let second = world.add_window(rect(2, 0, 2, 2)).unwrap();

        assert!(matches!(
            world.move_window(second, GridPoint::new(1, 0)),
            Err(WorldError::Occupied(_))
        ));
        assert_eq!(
            world.toggle_floating(second).unwrap(),
            GridConstraint::Floating
        );
        world.move_window(second, GridPoint::new(1, 0)).unwrap();
        assert!(world
            .window(first)
            .unwrap()
            .rect()
            .overlaps(world.window(second).unwrap().rect()));
        assert!(matches!(
            world.toggle_floating(second),
            Err(WorldError::Occupied(_))
        ));
    }

    #[test]
    fn fullscreen_and_maximize_restore_the_original_world_rect() {
        let mut world = world();
        let id = world.add_window(rect(-2, 1, 2, 2)).unwrap();
        let original = world.window(id).unwrap().rect();

        assert_eq!(
            world.toggle_fullscreen(id).unwrap(),
            Presentation::Fullscreen
        );
        assert_eq!(world.window(id).unwrap().rect(), rect(0, 0, 4, 4));
        assert!(matches!(
            world.move_window_by(id, 1, 0),
            Err(WorldError::PresentedWindow(_))
        ));
        assert_eq!(world.toggle_fullscreen(id).unwrap(), Presentation::Normal);
        assert_eq!(world.window(id).unwrap().rect(), original);

        assert_eq!(world.toggle_maximized(id).unwrap(), Presentation::Maximized);
        assert_eq!(world.toggle_maximized(id).unwrap(), Presentation::Normal);
        assert_eq!(world.window(id).unwrap().rect(), original);
    }

    #[test]
    fn fullscreen_rejects_shared_camera_actions_without_changing_camera() {
        let mut world = world();
        let fullscreen = world.add_window(rect(0, 0, 2, 2)).unwrap();
        let target = world.add_window(rect(4, 0, 2, 2)).unwrap();
        world.toggle_fullscreen(fullscreen).unwrap();
        let camera = *world.camera();

        for action in [
            Action::CameraStep(Direction::Right),
            Action::CameraNudge(Direction::Down),
            Action::CameraPan {
                delta_x: 0.5,
                delta_y: -0.5,
            },
            Action::CameraTo(target),
            Action::CameraCenter(target),
            Action::CameraFollow(target),
            Action::CameraZoom(0.5),
        ] {
            assert_eq!(
                world.apply(action),
                Err(WorldError::CameraLockedByFullscreen(fullscreen))
            );
            assert_eq!(*world.camera(), camera);
        }

        world.toggle_fullscreen(fullscreen).unwrap();
        assert!(world.apply(Action::CameraStep(Direction::Right)).is_ok());
    }

    #[test]
    fn runtime_properties_override_config_and_clear_back_to_it() {
        let mut world = world();
        let first = world.add_window(rect(0, 0, 1, 1)).unwrap();
        let second = world.add_window(rect(2, 0, 1, 1)).unwrap();
        world
            .set_config_window_property(first, WindowProperty::Opacity(0.8))
            .unwrap();
        world
            .set_config_window_property(second, WindowProperty::Opacity(0.8))
            .unwrap();
        world
            .apply(Action::SetWindowProperty {
                id: first,
                property: WindowProperty::Opacity(0.4),
            })
            .unwrap();

        assert_opacity(
            world.window(first).unwrap().effective_properties().opacity,
            0.4,
        );
        assert_opacity(
            world.window(second).unwrap().effective_properties().opacity,
            0.8,
        );

        world
            .apply(Action::ClearWindowProperty {
                id: first,
                kind: WindowPropertyKind::Opacity,
            })
            .unwrap();
        assert_opacity(
            world.window(first).unwrap().effective_properties().opacity,
            0.8,
        );
    }

    #[test]
    fn replacing_config_properties_preserves_runtime_overrides() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world
            .set_runtime_window_property(id, WindowProperty::Opacity(0.4))
            .unwrap();

        world
            .replace_config_window_properties(
                id,
                &[WindowProperty::Opacity(0.7), WindowProperty::Blur(true)],
            )
            .unwrap();

        let properties = world.window(id).unwrap().effective_properties();
        assert_opacity(properties.opacity, 0.4);
        assert!(properties.blur);
        world
            .clear_runtime_window_property(id, WindowPropertyKind::Opacity)
            .unwrap();
        assert_opacity(
            world.window(id).unwrap().effective_properties().opacity,
            0.7,
        );
    }

    #[test]
    fn floating_uses_the_same_property_precedence() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world
            .set_config_window_property(id, WindowProperty::Floating(true))
            .unwrap();
        assert_eq!(
            world.window(id).unwrap().grid_constraint(),
            GridConstraint::Floating
        );

        world
            .set_runtime_window_property(id, WindowProperty::Floating(false))
            .unwrap();
        assert_eq!(
            world.window(id).unwrap().grid_constraint(),
            GridConstraint::Tiled
        );
        world
            .clear_runtime_window_property(id, WindowPropertyKind::Floating)
            .unwrap();
        assert_eq!(
            world.window(id).unwrap().grid_constraint(),
            GridConstraint::Floating
        );
    }

    #[test]
    fn blur_uses_the_same_property_precedence() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 1, 1)).unwrap();
        world
            .set_config_window_property(id, WindowProperty::Blur(true))
            .unwrap();
        assert!(world.window(id).unwrap().effective_properties().blur);

        world
            .set_runtime_window_property(id, WindowProperty::Blur(false))
            .unwrap();
        assert!(!world.window(id).unwrap().effective_properties().blur);
        world
            .clear_runtime_window_property(id, WindowPropertyKind::Blur)
            .unwrap();
        assert!(world.window(id).unwrap().effective_properties().blur);
    }

    #[test]
    fn outputs_keep_independent_cameras_in_one_world() {
        let mut world = world();
        let primary = world.active_output();
        let mut second_camera = Camera::new(GridPoint::new(20, -8), GridSize::new(6, 3).unwrap());
        second_camera.set_zoom(0.5).unwrap();
        let second = world.add_output(second_camera).unwrap();
        let window = world.add_window(rect(2, 1, 2, 2)).unwrap();

        world.apply(Action::ActivateOutput(second)).unwrap();
        world.camera_nudge(Direction::Right).unwrap();

        assert_eq!(world.active_output(), second);
        assert_eq!(
            world.camera().position(),
            CameraPosition::new(21.0, -8.0).unwrap()
        );
        assert!((world.camera().zoom() - 0.5).abs() < f64::EPSILON);
        assert_eq!(world.window(window).unwrap().rect(), rect(2, 1, 2, 2));
        assert_eq!(
            world.camera_for_output(primary).unwrap().position(),
            CameraPosition::new(0.0, 0.0).unwrap()
        );

        world.apply(Action::CycleOutput).unwrap();
        assert_eq!(
            world.camera().position(),
            CameraPosition::new(0.0, 0.0).unwrap()
        );
        assert_eq!(
            world.camera_for_output(second).unwrap().position(),
            CameraPosition::new(21.0, -8.0).unwrap()
        );
    }

    #[test]
    fn activating_unknown_output_is_atomic() {
        let mut world = world();
        let active = world.active_output();
        let camera = *world.camera();

        assert!(matches!(
            world.activate_output(OutputId::from_raw(99)),
            Err(OutputError::Unknown(_))
        ));
        assert_eq!(world.active_output(), active);
        assert_eq!(*world.camera(), camera);
    }

    #[test]
    fn floating_window_keeps_a_continuous_world_position() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 2, 2)).unwrap();
        world.toggle_floating(id).unwrap();
        let origin = WorldPoint::new(-1.25, 3.625).unwrap();
        world
            .apply(Action::MoveWindowContinuous { id, origin })
            .unwrap();
        assert_eq!(world.window(id).unwrap().rect().origin(), origin);
    }

    #[test]
    fn tiled_window_rejects_a_continuous_off_grid_position() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 2, 2)).unwrap();
        assert_eq!(
            world.move_window_continuous(id, WorldPoint::new(0.5, 0.0).unwrap()),
            Err(WorldError::OffGrid(id))
        );
        assert_eq!(world.window(id).unwrap().rect(), rect(0, 0, 2, 2));
    }

    #[test]
    fn returning_to_tiled_snaps_continuous_position_to_grid() {
        let mut world = world();
        let id = world.add_window(rect(0, 0, 2, 2)).unwrap();
        world.toggle_floating(id).unwrap();
        world
            .move_window_continuous(id, WorldPoint::new(-1.25, 3.625).unwrap())
            .unwrap();
        assert_eq!(world.toggle_floating(id), Ok(GridConstraint::Tiled));
        assert_eq!(world.window(id).unwrap().rect(), rect(-1, 4, 2, 2));
    }
}
