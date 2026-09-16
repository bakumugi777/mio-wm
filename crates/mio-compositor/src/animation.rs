use crate::layout::ScreenRect;

#[derive(Clone, Copy, Debug)]
pub struct AnimatedValue {
    current: f64,
    target: f64,
}

impl AnimatedValue {
    pub const fn new(value: f64) -> Self {
        Self {
            current: value,
            target: value,
        }
    }

    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    pub fn advance(&mut self, seconds: f64, speed: f64) -> bool {
        if speed == 0.0 {
            self.current = self.target;
            return false;
        }
        let progress = 1.0 - (-12.0 * speed * seconds).exp();
        self.current += (self.target - self.current) * progress;
        if (self.target - self.current).abs() < 0.05 {
            self.current = self.target;
            false
        } else {
            true
        }
    }

    pub const fn current(self) -> f64 {
        self.current
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AnimatedRect {
    x: AnimatedValue,
    y: AnimatedValue,
    width: AnimatedValue,
    height: AnimatedValue,
}

impl AnimatedRect {
    pub fn new(rect: ScreenRect) -> Self {
        Self {
            x: AnimatedValue::new(f64::from(rect.x)),
            y: AnimatedValue::new(f64::from(rect.y)),
            width: AnimatedValue::new(f64::from(rect.width)),
            height: AnimatedValue::new(f64::from(rect.height)),
        }
    }

    pub fn set_target(&mut self, rect: ScreenRect) {
        self.x.set_target(f64::from(rect.x));
        self.y.set_target(f64::from(rect.y));
        self.width.set_target(f64::from(rect.width));
        self.height.set_target(f64::from(rect.height));
    }

    pub fn set_current(&mut self, rect: ScreenRect) {
        *self = Self::new(rect);
    }

    pub fn advance(&mut self, seconds: f64, speed: f64) -> bool {
        let mut active = self.x.advance(seconds, speed);
        active |= self.y.advance(seconds, speed);
        active |= self.width.advance(seconds, speed);
        active |= self.height.advance(seconds, speed);
        active
    }

    pub fn current(self) -> ScreenRect {
        ScreenRect {
            x: round_i32(self.x.current()),
            y: round_i32(self.y.current()),
            width: round_i32(self.width.current()).max(1),
            height: round_i32(self.height.current()).max(1),
        }
    }

    pub fn target(self) -> ScreenRect {
        ScreenRect {
            x: round_i32(self.x.target),
            y: round_i32(self.y.target),
            width: round_i32(self.width.target).max(1),
            height: round_i32(self.height.target).max(1),
        }
    }
}

fn round_i32(value: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value.round() as i32;
    rounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approaches_target_without_changing_it() {
        let mut value = AnimatedValue::new(0.0);
        value.set_target(100.0);
        assert!(value.advance(1.0 / 60.0, 1.0));
        assert!(value.current() > 0.0 && value.current() < 100.0);
        for _ in 0..120 {
            value.advance(1.0 / 60.0, 1.0);
        }
        assert!((value.current() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_speed_disables_animation() {
        let mut value = AnimatedValue::new(5.0);
        value.set_target(-20.0);
        assert!(!value.advance(1.0 / 60.0, 0.0));
        assert!((value.current() + 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rectangle_motion_uses_intermediate_render_geometry() {
        let start = ScreenRect {
            x: 0,
            y: 0,
            width: 400,
            height: 300,
        };
        let target = ScreenRect {
            x: 400,
            y: 200,
            width: 200,
            height: 150,
        };
        let mut rect = AnimatedRect::new(start);
        rect.set_target(target);

        assert!(rect.advance(1.0 / 60.0, 1.0));
        let current = rect.current();
        assert!(current.x > start.x && current.x < target.x);
        assert!(current.y > start.y && current.y < target.y);
        assert!(current.width < start.width && current.width > target.width);
        assert!(current.height < start.height && current.height > target.height);
    }
}
