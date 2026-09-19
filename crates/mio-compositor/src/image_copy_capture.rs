use std::{ptr, time::Duration};

use smithay::{
    backend::{allocator::Fourcc, renderer::ExportMem},
    delegate_image_capture_source, delegate_image_copy_capture, delegate_output_capture_source,
    output::WeakOutput,
    reexports::wayland_server::protocol::wl_shm,
    utils::{Buffer as BufferCoord, IsAlive, Rectangle, Transform},
    wayland::{
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, Frame, ImageCopyCaptureHandler,
            ImageCopyCaptureState, Session, SessionRef,
        },
        shm::with_buffer_contents_mut,
    },
};
use tracing::{info, warn};

use crate::state::MioState;

#[derive(Debug)]
pub(crate) struct PendingImageCopyCapture {
    frame: Frame,
    region: Rectangle<i32, BufferCoord>,
}

impl ImageCaptureSourceHandler for MioState {}

impl OutputCaptureSourceHandler for MioState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(
        &mut self,
        source: ImageCaptureSource,
        output: &smithay::output::Output,
    ) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for MioState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        if self.session_locked {
            return None;
        }
        let output = source.user_data().get::<WeakOutput>()?.upgrade()?;
        let mode = output.current_mode()?;
        Some(BufferConstraints {
            size: mode.size.to_logical(1).to_buffer(1, Transform::Normal),
            shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888],
            dma: None,
        })
    }

    fn new_session(&mut self, session: Session) {
        self.image_copy_capture_sessions
            .retain(|session| session.as_ref().alive());
        self.image_copy_capture_sessions.push(session);
        info!(target: "mio_compositor::image_copy_capture", "image-copy-capture session created");
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        if self.session_locked {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        }
        let Some(output) = session
            .source()
            .user_data()
            .get::<WeakOutput>()
            .and_then(WeakOutput::upgrade)
        else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        let Some(mode) = output.current_mode() else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        let Some(origin) = self
            .space
            .output_geometry(&output)
            .map(|geometry| geometry.loc)
        else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        let region = Rectangle::new(
            (origin.x, origin.y).into(),
            mode.size.to_logical(1).to_buffer(1, Transform::Normal),
        );
        self.pending_image_copy_captures
            .push(PendingImageCopyCapture { frame, region });
        info!(
            target: "mio_compositor::image_copy_capture",
            width = region.size.w,
            height = region.size.h,
            pending = self.pending_image_copy_captures.len(),
            "image-copy-capture queued"
        );
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }
}

delegate_image_capture_source!(MioState);
delegate_output_capture_source!(MioState);
delegate_image_copy_capture!(MioState);

impl MioState {
    pub(crate) fn has_pending_image_copy_captures(&self) -> bool {
        !self.pending_image_copy_captures.is_empty()
    }

    pub(crate) fn fulfill_image_copy_captures<R>(
        &mut self,
        renderer: &mut R,
        framebuffer: &R::Framebuffer<'_>,
        framebuffer_size: smithay::utils::Size<i32, BufferCoord>,
        timestamp: Duration,
    ) -> bool
    where
        R: ExportMem,
    {
        let fulfilled_any = !self.pending_image_copy_captures.is_empty();
        for pending in self.pending_image_copy_captures.drain(..) {
            let read_region = Rectangle::new(
                (
                    pending.region.loc.x,
                    framebuffer_size.h - pending.region.loc.y - pending.region.size.h,
                )
                    .into(),
                pending.region.size,
            );
            let result = renderer
                .copy_framebuffer(framebuffer, read_region, Fourcc::Argb8888)
                .and_then(|mapping| {
                    let bytes = renderer.map_texture(&mapping)?;
                    let copied =
                        with_buffer_contents_mut(&pending.frame.buffer(), |target, len, data| {
                            let Ok(width) = usize::try_from(pending.region.size.w) else {
                                return false;
                            };
                            let Ok(height) = usize::try_from(pending.region.size.h) else {
                                return false;
                            };
                            let Some(row_bytes) = width.checked_mul(4) else {
                                return false;
                            };
                            let Ok(target_stride) = usize::try_from(data.stride) else {
                                return false;
                            };
                            let Ok(offset) = usize::try_from(data.offset) else {
                                return false;
                            };
                            let Some(target_end) = target_stride
                                .checked_mul(height)
                                .and_then(|size| offset.checked_add(size))
                            else {
                                return false;
                            };
                            let Some(source_len) = row_bytes.checked_mul(height) else {
                                return false;
                            };
                            if target_stride < row_bytes
                                || target_end > len
                                || source_len > bytes.len()
                            {
                                return false;
                            }
                            for row in 0..height {
                                // SAFETY: Both source and target row bounds were checked above;
                                // Smithay keeps the SHM mapping valid for this callback.
                                unsafe {
                                    ptr::copy_nonoverlapping(
                                        bytes.as_ptr().add(row * row_bytes),
                                        target.add(offset + row * target_stride),
                                        row_bytes,
                                    );
                                }
                            }
                            true
                        })
                        .unwrap_or(false);
                    Ok(copied)
                });

            match result {
                Ok(true) => {
                    let width = pending.region.size.w;
                    let height = pending.region.size.h;
                    pending.frame.success(
                        Transform::Normal,
                        vec![Rectangle::new((0, 0).into(), (width, height).into())],
                        timestamp,
                    );
                    info!(
                        target: "mio_compositor::image_copy_capture",
                        width,
                        height,
                        timestamp_ns = timestamp.as_nanos(),
                        "image-copy-capture ready"
                    );
                }
                Ok(false) => pending.frame.fail(CaptureFailureReason::BufferConstraints),
                Err(error) => {
                    warn!(%error, "failed to read back image-copy-capture framebuffer");
                    pending.frame.fail(CaptureFailureReason::Unknown);
                }
            }
        }
        fulfilled_any
    }
}
