use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use mio_core::WindowId;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{AsRenderElements, Id},
            gles::{GlesRenderer, GlesTexture},
            utils::draw_render_elements,
            Bind, Frame, Offscreen, Renderer, Texture,
        },
    },
    utils::{Physical, Rectangle, Transform},
};

use crate::{
    config::WindowTransitionEffect,
    state::{MioState, RenderWindow, WindowTransition},
};

#[derive(Debug)]
pub(crate) struct WindowRenderSnapshot {
    pub(crate) id: Id,
    pub(crate) texture: GlesTexture,
    pub(crate) geometry: Rectangle<i32, Physical>,
}

#[derive(Debug)]
pub(crate) struct ClosingVisual {
    pub(crate) snapshot: WindowRenderSnapshot,
    pub(crate) started: Instant,
    pub(crate) duration: Duration,
    pub(crate) effect: WindowTransitionEffect,
}

pub(crate) fn capture_window_snapshots(
    state: &mut MioState,
    renderer: &mut GlesRenderer,
    snapshots: &mut HashMap<WindowId, WindowRenderSnapshot>,
) -> Result<(), smithay::backend::renderer::gles::GlesError> {
    for managed in &mut state.managed_windows {
        if !managed.ready_to_present || managed.transition == WindowTransition::Closing {
            continue;
        }
        let Some(mapped_location) = state.space.element_location(&managed.window) else {
            continue;
        };
        let geometry = smithay::desktop::space::SpaceElement::geometry(&managed.window);
        let bbox = smithay::desktop::space::SpaceElement::bbox(&managed.window);
        if bbox.size.w <= 0 || bbox.size.h <= 0 {
            continue;
        }
        let surface_origin = mapped_location - geometry.loc;
        let snapshot_geometry =
            Rectangle::new(surface_origin + bbox.loc, bbox.size).to_physical_precise_round(1.0);
        let size = bbox.size.to_buffer(1, Transform::Normal);
        let recreate = snapshots
            .get(&managed.id)
            .is_none_or(|snapshot| snapshot.texture.size() != size);
        if recreate {
            snapshots.insert(
                managed.id,
                WindowRenderSnapshot {
                    id: Id::new(),
                    texture: renderer.create_buffer(Fourcc::Abgr8888, size)?,
                    geometry: snapshot_geometry,
                },
            );
        }
        let snapshot = snapshots
            .get_mut(&managed.id)
            .expect("snapshot was inserted above");
        snapshot.geometry = snapshot_geometry;
        if !managed.snapshot_dirty && !recreate {
            continue;
        }
        let render_origin = (-bbox.loc.x, -bbox.loc.y).into();
        let elements: Vec<<RenderWindow as AsRenderElements<GlesRenderer>>::RenderElement> =
            <RenderWindow as AsRenderElements<GlesRenderer>>::render_elements(
                &managed.window,
                renderer,
                render_origin,
                1.0.into(),
                1.0,
            );
        let mut target = renderer.bind(&mut snapshot.texture)?;
        let mut frame =
            renderer.render(&mut target, bbox.size.to_physical(1), Transform::Normal)?;
        let damage = Rectangle::from_size(bbox.size.to_physical(1));
        frame.clear([0.0, 0.0, 0.0, 0.0].into(), &[damage])?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
        frame.finish().map(drop)?;
        managed.snapshot_dirty = false;
    }
    Ok(())
}

pub(crate) fn closing_visual_progress(elapsed: Duration, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32().max(f32::EPSILON);
    (1.0 - elapsed.as_secs_f32() / duration).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::closing_visual_progress;
    use std::time::Duration;

    #[test]
    fn closing_visual_progress_runs_backwards_and_clamps() {
        let duration = Duration::from_millis(400);
        assert_eq!(closing_visual_progress(Duration::ZERO, duration), 1.0);
        assert!(
            (closing_visual_progress(Duration::from_millis(200), duration) - 0.5).abs() < 0.001
        );
        assert_eq!(closing_visual_progress(duration, duration), 0.0);
        assert_eq!(
            closing_visual_progress(Duration::from_secs(1), duration),
            0.0
        );
    }
}
