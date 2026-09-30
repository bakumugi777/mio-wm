use crate::{
    EffectiveWindowProperties, GeometryError, GridRect, GridSize, WindowProperty,
    WindowPropertyKind, WindowPropertySet, WorldPoint, WorldRect,
};

/// Stable identity of a window for its lifetime in the World.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(u64);

impl WindowId {
    /// Reconstruct an identity received through an adapter boundary such as IPC.
    #[must_use]
    pub const fn from_u64(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// A window's logical state in the shared World.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    id: WindowId,
    rect: WorldRect,
    config_properties: WindowPropertySet,
    runtime_properties: WindowPropertySet,
    presentation: Presentation,
    restore_rect: Option<WorldRect>,
}

impl Window {
    pub(crate) fn new(id: WindowId, rect: GridRect) -> Self {
        Self {
            id,
            rect: rect.into(),
            config_properties: WindowPropertySet::default(),
            runtime_properties: WindowPropertySet::default(),
            presentation: Presentation::Normal,
            restore_rect: None,
        }
    }

    #[must_use]
    pub const fn id(&self) -> WindowId {
        self.id
    }

    #[must_use]
    pub const fn rect(&self) -> WorldRect {
        self.rect
    }

    #[must_use]
    pub fn grid_constraint(&self) -> GridConstraint {
        if self.effective_properties().floating {
            GridConstraint::Floating
        } else {
            GridConstraint::Tiled
        }
    }

    #[must_use]
    pub const fn presentation(&self) -> Presentation {
        self.presentation
    }

    #[must_use]
    pub fn effective_properties(&self) -> EffectiveWindowProperties {
        let defaults = EffectiveWindowProperties::default();
        EffectiveWindowProperties {
            opacity: self
                .runtime_properties
                .opacity()
                .or(self.config_properties.opacity())
                .unwrap_or(defaults.opacity),
            floating: self
                .runtime_properties
                .floating()
                .or(self.config_properties.floating())
                .unwrap_or(defaults.floating),
            blur: self
                .runtime_properties
                .blur()
                .or(self.config_properties.blur())
                .unwrap_or(defaults.blur),
        }
    }

    pub(crate) fn set_config_property(&mut self, property: WindowProperty) {
        self.config_properties.set(property);
    }

    pub(crate) fn clear_config_properties(&mut self) {
        self.config_properties = WindowPropertySet::default();
    }

    pub(crate) fn set_runtime_property(&mut self, property: WindowProperty) {
        self.runtime_properties.set(property);
    }

    pub(crate) fn clear_runtime_property(&mut self, kind: WindowPropertyKind) {
        self.runtime_properties.clear(kind);
    }

    pub(crate) fn set_rect(&mut self, rect: WorldRect) {
        self.rect = rect;
    }

    pub(crate) fn set_presentation(&mut self, presentation: Presentation, viewport: GridRect) {
        if presentation == self.presentation {
            if let Some(restore) = self.restore_rect.take() {
                self.rect = restore;
            }
            self.presentation = Presentation::Normal;
            return;
        }

        if self.presentation == Presentation::Normal {
            self.restore_rect = Some(self.rect);
        }
        self.rect = viewport.into();
        self.presentation = presentation;
    }

    pub(crate) fn move_to(&mut self, origin: WorldPoint) -> Result<(), GeometryError> {
        self.rect = self.rect.moved_to(origin)?;
        Ok(())
    }

    pub(crate) fn resize(&mut self, size: GridSize) -> Result<(), GeometryError> {
        self.rect = self.rect.resized(size)?;
        Ok(())
    }
}

/// Whether placement is constrained by the tiled World grid occupancy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridConstraint {
    Tiled,
    Floating,
}

/// A window's output-filling presentation state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presentation {
    Normal,
    Maximized,
    Fullscreen,
}
