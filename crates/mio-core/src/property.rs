/// A configurable or runtime-overridable Window property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowProperty {
    Opacity(f32),
    Floating(bool),
    Blur(bool),
}

/// Identifies a Window property without carrying a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowPropertyKind {
    Opacity,
    Floating,
    Blur,
}

impl WindowProperty {
    #[must_use]
    pub const fn kind(self) -> WindowPropertyKind {
        match self {
            Self::Opacity(_) => WindowPropertyKind::Opacity,
            Self::Floating(_) => WindowPropertyKind::Floating,
            Self::Blur(_) => WindowPropertyKind::Blur,
        }
    }
}

/// Values supplied by one precedence layer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowPropertySet {
    opacity: Option<f32>,
    floating: Option<bool>,
    blur: Option<bool>,
}

impl WindowPropertySet {
    pub(crate) fn set(&mut self, property: WindowProperty) {
        match property {
            WindowProperty::Opacity(value) => self.opacity = Some(value),
            WindowProperty::Floating(value) => self.floating = Some(value),
            WindowProperty::Blur(value) => self.blur = Some(value),
        }
    }

    pub(crate) fn clear(&mut self, kind: WindowPropertyKind) {
        match kind {
            WindowPropertyKind::Opacity => self.opacity = None,
            WindowPropertyKind::Floating => self.floating = None,
            WindowPropertyKind::Blur => self.blur = None,
        }
    }

    #[must_use]
    pub const fn opacity(self) -> Option<f32> {
        self.opacity
    }

    #[must_use]
    pub const fn floating(self) -> Option<bool> {
        self.floating
    }

    #[must_use]
    pub const fn blur(self) -> Option<bool> {
        self.blur
    }
}

/// Effective Window properties after precedence resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectiveWindowProperties {
    pub opacity: f32,
    pub floating: bool,
    pub blur: bool,
}

impl Default for EffectiveWindowProperties {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            floating: false,
            blur: false,
        }
    }
}
