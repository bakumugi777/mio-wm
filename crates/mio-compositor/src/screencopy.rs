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
    utils::{Buffer as BufferCoord, Rectangle, Size},
    wayland::shm::{with_buffer_contents, with_buffer_contents_mut},
};
use tracing::warn;

use crate::state::MioState;

const MANAGER_VERSION: u32 = 3;
const BYTES_PER_PIXEL: i32 = 4;

#[derive(Debug)]
pub(crate) struct FrameData {
    region: Rectangle<i32, BufferCoord>,
    output_origin: smithay::utils::Point<i32, BufferCoord>,
    used: Mutex<bool>,
}

#[derive(Debug)]
pub(crate) struct PendingScreencopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    region: Rectangle<i32, BufferCoord>,
    output_origin: smithay::utils::Point<i32, BufferCoord>,
    with_damage: bool,
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
        data_init.init(resource, ());
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
                overlay_cursor: _,
                output,
            } => state.create_screencopy_frame(frame, &output, None, data_init),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor: _,
                output,
                x,
                y,
                width,
                height,
            } => state.create_screencopy_frame(
                frame,
                &output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
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
        requested_region: Option<Rectangle<i32, BufferCoord>>,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = Output::from_resource(output_resource)
            .filter(|output| self.space.outputs().any(|candidate| candidate == output));
        let output_size = output
            .as_ref()
            .and_then(Output::current_mode)
            .map(|mode| Size::from((mode.size.w, mode.size.h)))
            .unwrap_or_default();
        let output_origin = output
            .as_ref()
            .and_then(|output| self.space.output_geometry(output))
            .map(|geometry| (geometry.loc.x, geometry.loc.y).into())
            .unwrap_or_default();
        let bounds = Rectangle::from_size(output_size);
        let region = requested_region
            .and_then(|region| bounds.intersection(region))
            .unwrap_or_else(|| {
                if requested_region.is_some() {
                    Rectangle::from_size(Size::from((0, 0)))
                } else {
                    bounds
                }
            });

        let frame = data_init.init(
            frame,
            FrameData {
                region,
                output_origin,
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
        });
    }

    pub(crate) fn fulfill_screencopies<R>(
        &mut self,
        renderer: &mut R,
        framebuffer: &R::Framebuffer<'_>,
        framebuffer_size: Size<i32, BufferCoord>,
        timestamp: Duration,
    ) -> bool
    where
        R: ExportMem,
    {
        let fulfilled_any = !self.pending_screencopies.is_empty();
        for pending in self.pending_screencopies.drain(..) {
            if !pending.frame.is_alive() {
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
                }
                Ok(false) => pending.frame.failed(),
                Err(error) => {
                    warn!(%error, "failed to read back screencopy framebuffer");
                    pending.frame.failed();
                }
            }
        }
        fulfilled_any
    }
}
