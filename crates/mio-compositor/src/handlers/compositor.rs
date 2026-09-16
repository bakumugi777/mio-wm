use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::state::{MioClientState, MioState};

use super::{layer_shell, xdg_shell};

impl CompositorHandler for MioState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<MioClientState>()
            .expect("all accepted clients have MioClientState")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(managed) = self.managed_windows.iter_mut().find(|managed| {
                managed
                    .window
                    .toplevel()
                    .is_some_and(|toplevel| toplevel.wl_surface() == &root)
            }) {
                managed.window.on_commit();
                managed.snapshot_dirty = true;
            }
        }
        layer_shell::handle_commit(self, surface);
        xdg_shell::handle_commit(self, surface);
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }
}

impl BufferHandler for MioState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for MioState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(MioState);
delegate_shm!(MioState);
