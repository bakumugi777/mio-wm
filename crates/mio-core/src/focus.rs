use crate::WindowId;

/// Logical keyboard focus, independent of camera visibility.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Focus {
    window: Option<WindowId>,
}

impl Focus {
    #[must_use]
    pub const fn window(self) -> Option<WindowId> {
        self.window
    }

    pub(crate) fn set(&mut self, window: Option<WindowId>) {
        self.window = window;
    }
}
