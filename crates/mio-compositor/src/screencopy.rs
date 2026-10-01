use std::{ptr, sync::Mutex, time::Duration};

use smithay::{
    backend::{allocator::Fourcc, renderer::ExportMem},
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            backend::{ClientId, GlobalId},
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Buffer as BufferCoord, Logical, Physical, Rectangle, Scale, Size, Transform},
    wayland::shm::{with_buffer_contents, with_buffer_contents_mut},
};
use tracing::{info, warn};

use crate::state::MioState;

const MANAGER_VERSION: u32 = 3;
const BYTES_PER_PIXEL: i32 = 4;

#[derive(Debug)]
pub(crate) struct FrameData {
    region: Rectangle<i32, BufferCoord>,
    output_origin: smithay::utils::Point<i32, BufferCoord>,
    overlay_cursor: bool,
    used: Mutex<bool>,
}

#[derive(Debug)]
pub(crate) struct PendingScreencopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    region: Rectangle<i32, BufferCoord>,
    output_origin: smithay::utils::Point<i32, BufferCoord>,
    with_damage: bool,
    overlay_cursor: bool,
}

pub(crate) fn create_global(display: &DisplayHandle) -> GlobalId {
    display.create_global::<MioState, ZwlrScreencopyManagerV1, _>(MANAGER_VERSION, ())
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for MioState {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        info!(
            target: "mio_compositor::screencopy",
            version = resource.version(),
            "screencopy client bound"
        );
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for MioState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                state.create_screencopy_frame(frame, &output, None, overlay_cursor != 0, data_init);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => state.create_screencopy_frame(
                frame,
                &output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
                overlay_cursor != 0,
                data_init,
            ),
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl MioState {
    fn create_screencopy_frame(
        &self,
        frame: New<ZwlrScreencopyFrameV1>,
        output_resource: &WlOutput,
        requested_region: Option<Rectangle<i32, Logical>>,
        overlay_cursor: bool,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = Output::from_resource(output_resource)
            .filter(|output| self.space.outputs().any(|candidate| candidate == output));
        let output_size = output
            .as_ref()
            .and_then(Output::current_mode)
            .map(|mode| Size::from((mode.size.w, mode.size.h)))
            .unwrap_or_default();
        let logical_geometry = output
            .as_ref()
            .and_then(|output| self.space.output_geometry(output))
            .unwrap_or_default();
        let scale = output
            .as_ref()
            .map_or(1.0, |output| output.current_scale().fractional_scale());
        let transform = output
            .as_ref()
            .map_or(Transform::Normal, Output::current_transform);
        let physical_origin = logical_geometry
            .loc
            .to_physical_precise_round::<f64, i32>(scale);
        let output_origin = (physical_origin.x, physical_origin.y).into();
        let logical_bounds = Rectangle::from_size(logical_geometry.size);
        let region = requested_region
            .and_then(|region| logical_bounds.intersection(region))
            .map_or_else(
                || {
                    if requested_region.is_some() {
                        Rectangle::from_size(Size::from((0, 0)))
                    } else {
                        Rectangle::from_size(output_size)
                    }
                },
                |region| logical_region_to_buffer(region, output_size, scale, transform),
            );
        info!(
            target: "mio_compositor::screencopy",
            requested_x = requested_region.map(|region| region.loc.x),
            requested_y = requested_region.map(|region| region.loc.y),
            requested_width = requested_region.map(|region| region.size.w),
            requested_height = requested_region.map(|region| region.size.h),
            logical_width = logical_geometry.size.w,
            logical_height = logical_geometry.size.h,
            scale,
            ?transform,
            buffer_x = region.loc.x,
            buffer_y = region.loc.y,
            buffer_width = region.size.w,
            buffer_height = region.size.h,
            "screencopy region resolved"
        );

        let frame = data_init.init(
            frame,
            FrameData {
                region,
                output_origin,
                overlay_cursor,
                used: Mutex::new(false),
            },
        );
        if region.size.w <= 0 || region.size.h <= 0 {
            frame.failed();
            return;
        }
        let (Ok(width), Ok(height)) = (u32::try_from(region.size.w), u32::try_from(region.size.h))
        else {
            frame.failed();
            return;
        };
        let Some(stride) = region
            .size
            .w
            .checked_mul(BYTES_PER_PIXEL)
            .and_then(|stride| u32::try_from(stride).ok())
        else {
            frame.failed();
            return;
        };
        frame.buffer(wl_shm::Format::Argb8888, width, height, stride);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

fn logical_region_to_buffer(
    region: Rectangle<i32, Logical>,
    output_size: Size<i32, BufferCoord>,
    scale: f64,
    transform: Transform,
) -> Rectangle<i32, BufferCoord> {
    let transformed_size = transform.transform_size(output_size);
    let transformed_physical_size =
        Size::<i32, Physical>::from((transformed_size.w, transformed_size.h));
    let physical_region = region.to_physical_precise_round::<f64, i32>(Scale::from(scale));
    let physical_bounds = Rectangle::from_size(transformed_physical_size);
    let Some(clamped) = physical_region.intersection(physical_bounds) else {
        return Rectangle::from_size((0, 0).into());
    };
    let untransformed = transform
        .invert()
        .transform_rect_in(clamped, &transformed_physical_size);
    Rectangle::new(
        (untransformed.loc.x, untransformed.loc.y).into(),
        (untransformed.size.w, untransformed.size.h).into(),
    )
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for MioState {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                state.queue_screencopy(frame, buffer, data, false);
            }
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                state.queue_screencopy(frame, buffer, data, true);
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: ClientId,
        _resource: &ZwlrScreencopyFrameV1,
        _data: &FrameData,
    ) {
    }
}

impl MioState {
    fn queue_screencopy(
        &mut self,
        frame: &ZwlrScreencopyFrameV1,
        buffer: WlBuffer,
        data: &FrameData,
        with_damage: bool,
    ) {
        if self.session_locked {
            frame.failed();
            return;
        }
        let Ok(mut used) = data.used.lock() else {
            frame.failed();
            return;
        };
        if *used {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "screencopy frame has already been used",
            );
            return;
        }
        *used = true;

        let valid = with_buffer_contents(&buffer, |_, len, buffer_data| {
            let bounds = usize::try_from(buffer_data.offset).ok().and_then(|offset| {
                usize::try_from(buffer_data.stride)
                    .ok()?
                    .checked_mul(usize::try_from(buffer_data.height).ok()?)?
                    .checked_add(offset)
            });
            buffer_data.format == wl_shm::Format::Argb8888
                && buffer_data.width == data.region.size.w
                && buffer_data.height == data.region.size.h
                && buffer_data.stride == data.region.size.w * BYTES_PER_PIXEL
                && bounds.is_some_and(|end| end <= len)
        })
        .unwrap_or(false);
        if !valid {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "buffer does not match the advertised SHM format and size",
            );
            return;
        }

        self.pending_screencopies.push(PendingScreencopy {
            frame: frame.clone(),
            buffer,
            region: data.region,
            output_origin: data.output_origin,
            with_damage,
            overlay_cursor: data.overlay_cursor,
        });
        info!(
            target: "mio_compositor::screencopy",
            with_damage,
            width = data.region.size.w,
            height = data.region.size.h,
            pending = self.pending_screencopies.len(),
            "screencopy queued"
        );
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }

    pub(crate) fn fulfill_screencopies<R>(
        &mut self,
        renderer: &mut R,
        framebuffer: &R::Framebuffer<'_>,
        framebuffer_size: Size<i32, BufferCoord>,
        timestamp: Duration,
        cursor_filter: Option<bool>,
    ) -> bool
    where
        R: ExportMem,
    {
        let mut remaining = Vec::new();
        let mut fulfilled_any = false;
        for pending in std::mem::take(&mut self.pending_screencopies) {
            if cursor_filter.is_some_and(|expected| pending.overlay_cursor != expected) {
                remaining.push(pending);
                continue;
            }
            fulfilled_any = true;
            if !pending.frame.is_alive() {
                if pending.buffer.is_alive() {
                    pending.buffer.release();
                }
                continue;
            }
            let read_region = Rectangle::new(
                (
                    pending.output_origin.x.saturating_add(pending.region.loc.x),
                    framebuffer_size.h
                        - pending.output_origin.y
                        - pending.region.loc.y
                        - pending.region.size.h,
                )
                    .into(),
                pending.region.size,
            );
            let result = renderer
                .copy_framebuffer(framebuffer, read_region, Fourcc::Argb8888)
                .and_then(|mapping| {
                    let bytes = renderer.map_texture(&mapping)?;
                    let copied = with_buffer_contents_mut(&pending.buffer, |target, _, data| {
                        let Some(len) = usize::try_from(data.stride).ok().and_then(|stride| {
                            stride.checked_mul(usize::try_from(data.height).ok()?)
                        }) else {
                            return false;
                        };
                        let Ok(offset) = usize::try_from(data.offset) else {
                            return false;
                        };
                        // SAFETY: Smithay validated this writable SHM mapping for the duration
                        // of the callback, and queue_screencopy checked offset + len bounds.
                        unsafe {
                            ptr::copy_nonoverlapping(bytes.as_ptr(), target.add(offset), len);
                        }
                        true
                    })
                    .unwrap_or(false);
                    Ok(copied)
                });

            match result {
                Ok(true) => {
                    if pending.with_damage {
                        let width = u32::try_from(pending.region.size.w).unwrap_or_default();
                        let height = u32::try_from(pending.region.size.h).unwrap_or_default();
                        pending.frame.damage(0, 0, width, height);
                    }
                    pending
                        .frame
                        .flags(zwlr_screencopy_frame_v1::Flags::empty());
                    let seconds = timestamp.as_secs();
                    let seconds_hi = u32::try_from(seconds >> 32).unwrap_or_default();
                    let seconds_lo =
                        u32::try_from(seconds & u64::from(u32::MAX)).unwrap_or_default();
                    pending
                        .frame
                        .ready(seconds_hi, seconds_lo, timestamp.subsec_nanos());
                    info!(
                        target: "mio_compositor::screencopy",
                        with_damage = pending.with_damage,
                        width = pending.region.size.w,
                        height = pending.region.size.h,
                        timestamp_ns = timestamp.as_nanos(),
                        "screencopy ready"
                    );
                }
                Ok(false) => pending.frame.failed(),
                Err(error) => {
                    warn!(%error, "failed to read back screencopy framebuffer");
                    pending.frame.failed();
                }
            }
            // Completing a screencopy frame does not release its wl_buffer. Portal
            // clients wait for this event before recycling their PipeWire/SHM buffer.
            if pending.buffer.is_alive() {
                pending.buffer.release();
            }
        }
        self.pending_screencopies = remaining;
        fulfilled_any
    }

    pub(crate) fn has_pending_screencopy_variant(&self, overlay_cursor: bool) -> bool {
        self.pending_screencopies
            .iter()
            .any(|pending| pending.overlay_cursor == overlay_cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_screencopy_region_uses_fractional_output_scale() {
        let region = Rectangle::<i32, Logical>::new((80, 40).into(), (400, 200).into());

        let buffer_region =
            logical_region_to_buffer(region, Size::from((1920, 1080)), 1.25, Transform::Normal);

        assert_eq!(buffer_region.loc, (100, 50).into());
        assert_eq!(buffer_region.size, (500, 250).into());
    }

    #[test]
    fn logical_screencopy_region_is_mapped_back_into_rotated_framebuffer() {
        // A 1920x1080 framebuffer rotated by 90 degrees exposes a
        // 864x1536 logical output at scale 1.25.
        let region = Rectangle::<i32, Logical>::new((80, 40).into(), (400, 200).into());

        let buffer_region =
            logical_region_to_buffer(region, Size::from((1920, 1080)), 1.25, Transform::_90);

        assert_eq!(buffer_region.loc, (50, 480).into());
        assert_eq!(buffer_region.size, (250, 500).into());
    }

    #[test]
    fn logical_screencopy_region_is_clipped_in_transformed_physical_space() {
        let region = Rectangle::<i32, Logical>::new((1500, 840).into(), (100, 100).into());

        let buffer_region =
            logical_region_to_buffer(region, Size::from((1920, 1080)), 1.25, Transform::Normal);

        assert_eq!(buffer_region.loc, (1875, 1050).into());
        assert_eq!(buffer_region.size, (45, 30).into());
    }
}
