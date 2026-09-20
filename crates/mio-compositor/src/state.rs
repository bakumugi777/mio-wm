use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    error::Error,
    ffi::OsString,
    ops::Deref,
    rc::Rc,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use mio_core::{
    Action, Camera, Direction, GridPoint, OutputId, Presentation, WindowId, WindowProperty, World,
    WorldPoint, WorldRect,
};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{
            element::{utils::RescaleRenderElement, AsRenderElements, Element, Id},
            gles::GlesRenderer,
        },
    },
    desktop::{
        layer_map_for_output, space::SpaceElement, LayerSurface, PopupManager, Space, Window,
        WindowSurfaceType,
    },
    input::{
        keyboard::XkbConfig,
        pointer::{CursorIcon, CursorImageStatus},
        Seat, SeatState,
    },
    reexports::{
        calloop::{
            channel::Sender, generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction,
        },
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{
        Clock, IsAlive, Logical, Monotonic, Physical, Point, Rectangle, Scale, Serial,
        SERIAL_COUNTER,
    },
    wayland::{
        alpha_modifier::AlphaModifierState,
        compositor::{
            with_surface_tree_downward, CompositorClientState, CompositorState, TraversalAction,
        },
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufGlobal, DmabufState, ImportNotifier},
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session},
        input_method::InputMethodManagerState,
        keyboard_shortcuts_inhibit::{KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor},
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::data_device::DataDeviceState,
        selection::{primary_selection::PrimarySelectionState, wlr_data_control::DataControlState},
        session_lock::{LockSurface, SessionLockManagerState, SessionLocker},
        shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer, WlrLayerShellState},
        shell::xdg::dialog::XdgDialogState,
        shell::xdg::{decoration::XdgDecorationState, ShellClient, ToplevelSurface, XdgShellState},
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_activation::XdgActivationState,
        xdg_foreign::XdgForeignState,
        xdg_toplevel_icon::XdgToplevelIconManager,
    },
};
use tracing::{debug, error, info, warn};

use crate::{
    animation::{AnimatedRect, AnimatedValue},
    config::{ConfigManager, WindowTransitionEffect},
    effects::{
        BackdropBlurElement, BlurOptions, BlurPrograms, CursorWakeTrail, FocusGlowElement,
        FocusGlowOptions, RoundedElement, ShadowElement, ShadowOptions, WindowBorderElement,
        WindowBorderOptions,
    },
    layout::{inset_screen_rect, world_to_screen, ScreenRect},
    CalloopData,
};

pub type StateResult<T> = Result<T, Box<dyn Error>>;
const OPENING_CONTENT_GRACE_SECONDS: f64 = 0.35;
const OPENING_CONTENT_COMMIT_COUNT: u8 = 3;

fn window_transition_speed(animation_speed: f64, duration_ms: u32) -> f64 {
    let duration = f64::from(duration_ms) / 1000.0;
    animation_speed * 20.0_f64.ln() / (12.0 * duration)
}

fn create_toplevel_icon_manager(display: &DisplayHandle) -> XdgToplevelIconManager {
    let mut manager = XdgToplevelIconManager::new::<MioState>(display);
    for size in [32, 64, 128, 256] {
        manager.add_icon_size(size);
    }
    manager
}

pub struct MioState {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub space: Space<RenderWindow>,
    pub loop_signal: LoopSignal,
    pub(crate) redraw_sender: Option<Sender<()>>,
    pub compositor_state: CompositorState,
    pub _alpha_modifier_state: AlphaModifierState,
    pub _content_type_state: ContentTypeState,
    pub _xdg_toplevel_icon_manager: XdgToplevelIconManager,
    pub _cursor_shape_manager_state: CursorShapeManagerState,
    pub xdg_shell_state: XdgShellState,
    pub _xdg_dialog_state: XdgDialogState,
    pub(crate) xdg_clients: Vec<XdgClientPing>,
    pub _xdg_decoration_state: XdgDecorationState,
    pub _presentation_state: PresentationState,
    pub layer_shell_state: WlrLayerShellState,
    pub _idle_inhibit_state: IdleInhibitManagerState,
    pub _viewporter_state: ViewporterState,
    pub _fractional_scale_manager_state: FractionalScaleManagerState,
    pub _text_input_manager_state: TextInputManagerState,
    pub _input_method_manager_state: InputMethodManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub keyboard_shortcut_inhibitors: Vec<KeyboardShortcutsInhibitor>,
    pub _relative_pointer_manager_state: RelativePointerManagerState,
    pub _pointer_constraints_state: PointerConstraintsState,
    pub dmabuf_state: DmabufState,
    pub(crate) dmabuf_global: Option<DmabufGlobal>,
    pub(crate) pending_dmabuf_imports: Vec<(Dmabuf, ImportNotifier)>,
    pub _virtual_keyboard_manager_state: VirtualKeyboardManagerState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_foreign_state: XdgForeignState,
    pub session_lock_state: SessionLockManagerState,
    pub session_locked: bool,
    pub pending_session_lock: Option<SessionLocker>,
    pub session_lock_surfaces: Vec<(smithay::output::Output, LockSurface)>,
    pub(crate) image_copy_capture_state: ImageCopyCaptureState,
    pub(crate) _image_capture_source_state: ImageCaptureSourceState,
    pub(crate) output_capture_source_state: OutputCaptureSourceState,
    pub(crate) image_copy_capture_sessions: Vec<Session>,
    pub shm_state: ShmState,
    pub _single_pixel_buffer_state: SinglePixelBufferState,
    pub _output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub popups: PopupManager,
    pub hidden_fullscreen_layers: Vec<(smithay::output::Output, LayerSurface)>,
    pub(crate) layer_keyboard_interactivity: HashMap<WlSurface, KeyboardInteractivity>,
    pub idle_inhibiting_surfaces: Vec<WlSurface>,
    pub seat: Seat<Self>,
    pub world: World,
    pub managed_windows: Vec<ManagedWindow>,
    /// Renderer-adapter notifications for toplevels which disappeared without
    /// going through Mio's `CloseWindow` action. The renderer pairs these with
    /// its last per-window snapshot; this is deliberately not World state.
    pub(crate) destroyed_window_transitions: Vec<DestroyedWindowTransition>,
    pub(crate) outputs: BTreeMap<OutputId, smithay::output::Output>,
    pub output_size: Option<(i32, i32)>,
    pub config: ConfigManager,
    pub(crate) config_error: Option<String>,
    pub(crate) pending_camera_drag: Option<PendingCameraDrag>,
    /// RMB-wheel Camera zoom accumulated until the next rendered frame.
    /// This avoids recalculating every intermediate layout when libinput
    /// delivers several wheel events before DRM can present another frame.
    pub(crate) pending_camera_zoom_scroll: f64,
    pub(crate) pending_window_drag: Option<PendingWindowDrag>,
    pub(crate) pending_window_resize: Option<PendingWindowResize>,
    pub(crate) pending_floating_chord: Option<PendingFloatingChord>,
    pub(crate) pending_edge_placement: Option<(Direction, u32)>,
    pub(crate) pointer_buttons_held: u8,
    pub(crate) pending_close_click: Option<PendingCloseClick>,
    pub(crate) closing_pointer_chord: bool,
    pub(crate) pending_pointer_click: Option<PendingPointerClick>,
    pub(crate) suppressed_window_drag_releases: u8,
    pub(crate) suppressed_focus_click: bool,
    pub cursor_image_status: CursorImageStatus,
    pub(crate) cursor_override: Option<CursorIcon>,
    pub(crate) cursor_hidden_by_activity: bool,
    pub(crate) last_pointer_activity: Instant,
    pub(crate) dnd_icon: Option<DndIcon>,
    pub(crate) last_host_pointer_position: Option<Point<f64, Logical>>,
    pub(crate) cursor_wake: CursorWakeTrail,
    pub(crate) cursor_wake_override: Option<bool>,
    pub(crate) pending_screencopies: Vec<crate::screencopy::PendingScreencopy>,
    pub(crate) pending_image_copy_captures: Vec<crate::image_copy_capture::PendingImageCopyCapture>,
    pub(crate) xwayland_satellite: Option<std::process::Child>,
    pub(crate) xwayland_display: Option<String>,
    pub(crate) spawned_commands: Vec<std::process::Child>,
    pub(crate) pending_startup_commands: Vec<Vec<std::ffi::OsString>>,
    pub(crate) backend_name: &'static str,
    pub(crate) ipc_socket_path: Option<std::path::PathBuf>,
    pub(crate) presentation_clock: Clock<Monotonic>,
    pub(crate) presentation_sequence: u64,
    focus_indicator_window: Option<WindowId>,
    previous_focus_indicator: Option<(WindowId, Instant)>,
    focus_indicator_started_at: Instant,
    last_animation_frame: Instant,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DestroyedWindowTransition {
    pub(crate) id: WindowId,
    pub(crate) effect: WindowTransitionEffect,
    pub(crate) duration: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingCameraDrag {
    pub(crate) button: u32,
    pub(crate) start: Point<f64, Logical>,
    pub(crate) window: Option<WindowId>,
    pub(crate) start_camera_x: f64,
    pub(crate) start_camera_y: f64,
    pub(crate) start_zoom: f64,
    pub(crate) press_time: u32,
    pub(crate) dragging: bool,
    pub(crate) wheel_used: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingWindowDrag {
    pub(crate) id: WindowId,
    pub(crate) buttons: u8,
    pub(crate) origin: WorldPoint,
    pub(crate) start: Point<f64, Logical>,
    pub(crate) current: Point<f64, Logical>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingCloseClick {
    pub(crate) id: WindowId,
    pub(crate) position: Point<f64, Logical>,
    pub(crate) deadline: Instant,
    pub(crate) clicks: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // Four independent physical edges are the value itself.
pub(crate) struct ResizeEdges {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) top: bool,
    pub(crate) bottom: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingWindowResize {
    pub(crate) id: WindowId,
    pub(crate) rect: WorldRect,
    pub(crate) screen: ScreenRect,
    pub(crate) edges: ResizeEdges,
    pub(crate) start: Point<f64, Logical>,
    pub(crate) current: Point<f64, Logical>,
    pub(crate) followers: Vec<ResizeFollowerPreview>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResizeFollowerPreview {
    pub(crate) id: WindowId,
    pub(crate) screen: ScreenRect,
    pub(crate) horizontal: bool,
    pub(crate) vertical: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingFloatingChord {
    pub(crate) id: WindowId,
    pub(crate) buttons_held: u8,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingPointerClick {
    pub(crate) button: u32,
    pub(crate) press_time: u32,
    pub(crate) release_time: u32,
    pub(crate) deadline: Instant,
    pub(crate) position: Point<f64, Logical>,
    pub(crate) window: Option<WindowId>,
    pub(crate) clicks: u8,
}

#[derive(Debug)]
pub(crate) struct DndIcon {
    pub(crate) surface: WlSurface,
    pub(crate) offset: Point<i32, Logical>,
}

#[derive(Debug)]
pub(crate) struct XdgClientPing {
    pub(crate) client: ShellClient,
    pub(crate) next_ping: Instant,
    pub(crate) deadline: Option<Instant>,
    pub(crate) timed_out: bool,
}

impl XdgClientPing {
    pub(crate) fn new(client: ShellClient, now: Instant, interval: Duration) -> Self {
        Self {
            client,
            next_ping: now + interval,
            deadline: None,
            timed_out: false,
        }
    }
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)] // Independent lifecycle facts; not an implicit mode.
pub struct ManagedWindow {
    pub id: WindowId,
    pub window: RenderWindow,
    return_focus_candidates: Vec<WindowId>,
    protocol_parent: Option<WindowId>,
    pub(crate) auto_floating: bool,
    pub(crate) preserve_committed_size: bool,
    pub(crate) ready_to_present: bool,
    pub(crate) transition: WindowTransition,
    pub(crate) snapshot_dirty: bool,
    initial_commit_count: u8,
    render_rect: Option<AnimatedRect>,
    virtual_render_rects: BTreeMap<OutputId, AnimatedRect>,
    render_opacity: AnimatedValue,
    transition_progress: AnimatedValue,
    opening_delay: f64,
    client_width: AnimatedValue,
    client_height: AnimatedValue,
    last_configured_size: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowTransition {
    Opening,
    Stable,
    Closing,
}

pub(crate) struct VirtualWindowView {
    pub window: RenderWindow,
    pub screen: ScreenRect,
    pub crop: Rectangle<i32, Physical>,
}

#[derive(Clone, Debug)]
pub struct RenderWindow {
    window: Window,
    opacity: Arc<RwLock<f32>>,
    scale: Arc<RwLock<Scale<f64>>>,
    floating: Arc<RwLock<bool>>,
    blur: Arc<RwLock<bool>>,
    blur_id: Id,
    blur_context: Rc<RefCell<Option<(BlurPrograms, BlurOptions)>>>,
    rounding_program: Rc<RefCell<Option<smithay::backend::renderer::gles::GlesTexProgram>>>,
    corner_radius: Arc<RwLock<u32>>,
    presentation: Arc<RwLock<Presentation>>,
    transition_progress: Arc<RwLock<f32>>,
    transition_effect: Arc<RwLock<i32>>,
    transition_direction: Arc<RwLock<i32>>,
    shadow_id: Id,
    shadow_context: Rc<
        RefCell<
            Option<(
                smithay::backend::renderer::gles::GlesPixelProgram,
                ShadowOptions,
            )>,
        >,
    >,
    focus_glow_id: Id,
    focus_glow_context: Rc<
        RefCell<
            Option<(
                smithay::backend::renderer::gles::GlesPixelProgram,
                FocusGlowOptions,
            )>,
        >,
    >,
    window_border_id: Id,
    window_border_context: Rc<
        RefCell<
            Option<(
                smithay::backend::renderer::gles::GlesPixelProgram,
                WindowBorderOptions,
            )>,
        >,
    >,
}

impl RenderWindow {
    fn new(window: Window) -> Self {
        Self {
            window,
            opacity: Arc::new(RwLock::new(1.0)),
            scale: Arc::new(RwLock::new(Scale::from(1.0))),
            floating: Arc::new(RwLock::new(false)),
            blur: Arc::new(RwLock::new(false)),
            blur_id: Id::new(),
            blur_context: Rc::new(RefCell::new(None)),
            rounding_program: Rc::new(RefCell::new(None)),
            corner_radius: Arc::new(RwLock::new(0)),
            presentation: Arc::new(RwLock::new(Presentation::Normal)),
            transition_progress: Arc::new(RwLock::new(1.0)),
            transition_effect: Arc::new(RwLock::new(WindowTransitionEffect::Water.shader_value())),
            transition_direction: Arc::new(RwLock::new(0)),
            shadow_id: Id::new(),
            shadow_context: Rc::new(RefCell::new(None)),
            focus_glow_id: Id::new(),
            focus_glow_context: Rc::new(RefCell::new(None)),
            window_border_id: Id::new(),
            window_border_context: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn set_opacity(&self, opacity: f32) {
        if let Ok(mut current) = self.opacity.write() {
            *current = opacity;
        }
    }

    pub(crate) fn set_transition_progress(&self, progress: f32, direction: i32) {
        if let Ok(mut current) = self.transition_progress.write() {
            *current = progress;
        }
        if let Ok(mut current) = self.transition_direction.write() {
            *current = direction;
        }
    }

    fn set_scale(&self, scale: Scale<f64>) {
        if let Ok(mut current) = self.scale.write() {
            *current = scale;
        }
    }

    pub(crate) fn scale(&self) -> Scale<f64> {
        self.scale
            .read()
            .map_or_else(|_| Scale::from(1.0), |value| *value)
    }

    fn set_floating(&self, floating: bool) {
        if let Ok(mut current) = self.floating.write() {
            *current = floating;
        }
    }

    fn set_blur(&self, blur: bool) {
        if let Ok(mut current) = self.blur.write() {
            *current = blur;
        }
    }

    fn set_presentation(&self, presentation: Presentation) {
        if let Ok(mut current) = self.presentation.write() {
            *current = presentation;
        }
    }

    pub(crate) fn blur_enabled(&self) -> bool {
        self.blur.read().is_ok_and(|value| *value)
    }

    pub(crate) fn blur_id(&self) -> Id {
        self.blur_id.clone()
    }

    pub(crate) fn set_blur_context(&self, programs: Option<&BlurPrograms>, options: BlurOptions) {
        *self.blur_context.borrow_mut() = programs.cloned().map(|programs| (programs, options));
    }

    pub(crate) fn set_rounding_context(
        &self,
        program: Option<&smithay::backend::renderer::gles::GlesTexProgram>,
        radius: u32,
        transition_effect: WindowTransitionEffect,
    ) {
        *self.rounding_program.borrow_mut() = program.cloned();
        if let Ok(mut value) = self.corner_radius.write() {
            *value = radius;
        }
        if let Ok(mut value) = self.transition_effect.write() {
            *value = transition_effect.shader_value();
        }
    }

    pub(crate) fn set_shadow_context(
        &self,
        program: Option<&smithay::backend::renderer::gles::GlesPixelProgram>,
        options: ShadowOptions,
    ) {
        *self.shadow_context.borrow_mut() = program.cloned().map(|program| (program, options));
    }

    pub(crate) fn set_focus_glow_context(
        &self,
        program: Option<&smithay::backend::renderer::gles::GlesPixelProgram>,
        options: Option<FocusGlowOptions>,
    ) {
        *self.focus_glow_context.borrow_mut() = program.cloned().zip(options);
    }

    pub(crate) fn set_window_border_context(
        &self,
        program: Option<&smithay::backend::renderer::gles::GlesPixelProgram>,
        options: Option<WindowBorderOptions>,
    ) {
        *self.window_border_context.borrow_mut() = program.cloned().zip(options);
    }
}

impl Deref for RenderWindow {
    type Target = Window;

    fn deref(&self) -> &Self::Target {
        &self.window
    }
}

impl PartialEq for RenderWindow {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window
    }
}

impl IsAlive for RenderWindow {
    fn alive(&self) -> bool {
        self.window.alive()
    }
}

impl SpaceElement for RenderWindow {
    fn bbox(&self) -> Rectangle<i32, Logical> {
        scale_logical_rect(SpaceElement::bbox(&self.window), self.scale())
    }

    fn geometry(&self) -> Rectangle<i32, Logical> {
        scale_logical_rect(SpaceElement::geometry(&self.window), self.scale())
    }

    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        let scale = self.scale();
        SpaceElement::is_in_input_region(
            &self.window,
            &point.upscale((1.0 / scale.x, 1.0 / scale.y)),
        )
    }

    fn z_index(&self) -> u8 {
        self.floating
            .read()
            .map_or(0, |floating| floating_z_index(*floating))
    }

    fn set_activate(&self, activated: bool) {
        SpaceElement::set_activate(&self.window, activated);
    }

    fn output_enter(&self, output: &smithay::output::Output, overlap: Rectangle<i32, Logical>) {
        SpaceElement::output_enter(&self.window, output, overlap);
    }

    fn output_leave(&self, output: &smithay::output::Output) {
        SpaceElement::output_leave(&self.window, output);
    }

    fn refresh(&self) {
        SpaceElement::refresh(&self.window);
    }
}

fn scale_logical_rect(rect: Rectangle<i32, Logical>, scale: Scale<f64>) -> Rectangle<i32, Logical> {
    rect.to_f64().upscale(scale).to_i32_round()
}

const fn floating_z_index(floating: bool) -> u8 {
    if floating {
        1
    } else {
        0
    }
}

type BaseWindowElement =
    RescaleRenderElement<<Window as AsRenderElements<GlesRenderer>>::RenderElement>;

smithay::backend::renderer::element::render_elements! {
    pub RenderWindowElement<=GlesRenderer>;
    Surface=BaseWindowElement,
    Rounded=RoundedElement<BaseWindowElement>,
    Blur=BackdropBlurElement,
    Shadow=ShadowElement,
    FocusGlow=FocusGlowElement,
    Border=WindowBorderElement,
}

impl AsRenderElements<GlesRenderer> for RenderWindow {
    type RenderElement = RenderWindowElement;

    #[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut GlesRenderer,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let opacity = self.opacity.read().map_or(1.0, |value| *value);
        let render_scale = self
            .scale
            .read()
            .map_or_else(|_| Scale::from(1.0), |value| *value);
        let fullscreen = self
            .presentation
            .read()
            .is_ok_and(|value| *value == Presentation::Fullscreen);
        let corner_radius = if fullscreen {
            0
        } else {
            self.corner_radius.read().map_or(0, |value| *value)
        };
        let transition_progress = self.transition_progress.read().map_or(1.0, |value| *value);
        let transition_effect = self.transition_effect.read().map_or(1, |value| *value);
        let transition_direction = self.transition_direction.read().map_or(0, |value| *value);
        let rounding_program = self.rounding_program.borrow().clone();
        let mut window_geometry = SpaceElement::geometry(self);
        window_geometry.loc += location.to_f64().to_logical(scale).to_i32_round();
        let clip = window_geometry.to_physical_precise_round(scale);
        let presentation_scale = render_scale.x.min(render_scale.y);
        let radius = presented_corner_radius(corner_radius, scale.x, presentation_scale);
        let popup_surface_ids = self
            .window
            .wl_surface()
            .map(|root| popup_surface_tree_ids(&root))
            .unwrap_or_default();
        let mut elements = self
            .window
            .render_elements::<<Window as AsRenderElements<GlesRenderer>>::RenderElement>(
                renderer,
                location,
                scale,
                alpha * opacity,
            )
            .into_iter()
            .map(|element| RescaleRenderElement::from_element(element, location, render_scale))
            .map(|element| {
                if let Some(program) = rounding_program.clone().filter(|_| {
                    should_round_surface_element(
                        popup_surface_ids.contains(element.id()),
                        corner_radius > 0 || transition_progress < 1.0,
                    )
                }) {
                    RenderWindowElement::Rounded(RoundedElement::new(
                        element,
                        program,
                        clip,
                        radius,
                        transition_progress,
                        transition_effect,
                        transition_direction,
                    ))
                } else {
                    RenderWindowElement::Surface(element)
                }
            })
            .collect::<Vec<_>>();
        if let Some((program, mut options)) = self.window_border_context.borrow().clone() {
            options.color[3] *= transition_progress;
            options.corner_radius = if fullscreen {
                0.0
            } else {
                options.corner_radius * presentation_scale as f32
            };
            elements.insert(
                0,
                RenderWindowElement::Border(WindowBorderElement::new(
                    self.window_border_id.clone(),
                    window_geometry,
                    program,
                    options,
                )),
            );
        }
        if let Some((program, options)) = self.focus_glow_context.borrow().clone() {
            elements.insert(
                0,
                RenderWindowElement::FocusGlow(FocusGlowElement::new(
                    self.focus_glow_id.clone(),
                    window_geometry,
                    program,
                    options,
                )),
            );
        }
        if self.blur_enabled() {
            if let Some((programs, options)) = self.blur_context.borrow().clone() {
                elements.push(RenderWindowElement::Blur(BackdropBlurElement::new(
                    self.blur_id(),
                    window_geometry,
                    programs,
                    options,
                    rounding_program
                        .clone()
                        .filter(|_| corner_radius > 0 || transition_progress < 1.0)
                        .map(|program| {
                            (
                                program,
                                radius,
                                transition_progress,
                                transition_effect,
                                transition_direction,
                            )
                        }),
                )));
            }
        }
        if let Some((program, mut options)) = self
            .shadow_context
            .borrow()
            .clone()
            .filter(|(_, options)| options.radius > 0.0 && options.color[3] > 0.0)
        {
            options.color[3] *= transition_progress;
            elements.push(RenderWindowElement::Shadow(ShadowElement::new(
                self.shadow_id.clone(),
                window_geometry,
                program,
                options,
            )));
        }
        elements.into_iter().map(C::from).collect()
    }
}

fn popup_surface_tree_ids(root: &WlSurface) -> Vec<Id> {
    let mut ids = Vec::new();
    for (popup, _) in PopupManager::popups_for_surface(root) {
        with_surface_tree_downward(
            popup.wl_surface(),
            (),
            |_, _, &()| TraversalAction::DoChildren(()),
            |surface, _, &()| {
                ids.push(Id::from_wayland_resource(surface));
            },
            |_, _, &()| true,
        );
    }
    ids
}

const fn should_round_surface_element(is_popup: bool, rounding_active: bool) -> bool {
    rounding_active && !is_popup
}

fn scaled_surface_origin(
    position: Point<f64, Logical>,
    window_local: Point<f64, Logical>,
    surface_offset: Point<i32, Logical>,
    scale: Scale<f64>,
) -> Point<f64, Logical> {
    let surface_local = window_local - surface_offset.to_f64();
    position - surface_local.upscale(scale)
}

impl MioState {
    #[allow(clippy::too_many_lines)]
    pub fn new(
        event_loop: &mut EventLoop<CalloopData>,
        display: Display<Self>,
        config: ConfigManager,
    ) -> StateResult<Self> {
        let display_handle = display.handle();
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new_with_capabilities::<Self>(
            &display_handle,
            [
                xdg_toplevel::WmCapabilities::Fullscreen,
                xdg_toplevel::WmCapabilities::Maximize,
            ],
        );
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&display_handle);
        let xdg_dialog_state = XdgDialogState::new::<Self>(&display_handle);
        let presentation_clock = Clock::<Monotonic>::new();
        let presentation_state =
            PresentationState::new::<Self>(&display_handle, presentation_clock.id() as u32);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&display_handle);
        let viewporter_state = ViewporterState::new::<Self>(&display_handle);
        let fractional_scale_manager_state =
            FractionalScaleManagerState::new::<Self>(&display_handle);
        let text_input_manager_state = TextInputManagerState::new::<Self>(&display_handle);
        let input_method_manager_state =
            InputMethodManagerState::new::<Self, _>(&display_handle, |_| true);
        let keyboard_shortcuts_inhibit_state =
            KeyboardShortcutsInhibitState::new::<Self>(&display_handle);
        let relative_pointer_manager_state =
            RelativePointerManagerState::new::<Self>(&display_handle);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&display_handle);
        let virtual_keyboard_manager_state =
            VirtualKeyboardManagerState::new::<Self, _>(&display_handle, |_| true);
        let xdg_activation_state = XdgActivationState::new::<Self>(&display_handle);
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&display_handle, |_| true);
        crate::screencopy::create_global(&display_handle);
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new::<Self>(&display_handle);
        let image_copy_capture_state = ImageCopyCaptureState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let data_control_state = DataControlState::new::<Self, _>(
            &display_handle,
            Some(&primary_selection_state),
            |_| true,
        );
        let mut seat = seat_state.new_wl_seat(&display_handle, "mio-winit");
        seat.add_keyboard(XkbConfig::default(), 200, 25)?;
        seat.add_pointer();
        seat.add_touch();

        let socket_name = Self::init_wayland_listener(display, event_loop)?;
        Ok(Self {
            start_time: Instant::now(),
            socket_name,
            display_handle: display_handle.clone(),
            space: Space::default(),
            loop_signal: event_loop.get_signal(),
            redraw_sender: None,
            compositor_state,
            _alpha_modifier_state: AlphaModifierState::new::<Self>(&display_handle),
            _content_type_state: ContentTypeState::new::<Self>(&display_handle),
            _xdg_toplevel_icon_manager: create_toplevel_icon_manager(&display_handle),
            _cursor_shape_manager_state: cursor_shape_manager_state,
            xdg_shell_state,
            _xdg_dialog_state: xdg_dialog_state,
            xdg_clients: Vec::new(),
            _xdg_decoration_state: xdg_decoration_state,
            _presentation_state: presentation_state,
            layer_shell_state,
            _idle_inhibit_state: idle_inhibit_state,
            _viewporter_state: viewporter_state,
            _fractional_scale_manager_state: fractional_scale_manager_state,
            _text_input_manager_state: text_input_manager_state,
            _input_method_manager_state: input_method_manager_state,
            keyboard_shortcuts_inhibit_state,
            keyboard_shortcut_inhibitors: Vec::new(),
            _relative_pointer_manager_state: relative_pointer_manager_state,
            _pointer_constraints_state: pointer_constraints_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            pending_dmabuf_imports: Vec::new(),
            _virtual_keyboard_manager_state: virtual_keyboard_manager_state,
            xdg_activation_state,
            xdg_foreign_state: XdgForeignState::new::<Self>(&display_handle),
            session_lock_state,
            session_locked: false,
            pending_session_lock: None,
            session_lock_surfaces: Vec::new(),
            image_copy_capture_state,
            _image_capture_source_state: image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_sessions: Vec::new(),
            shm_state: ShmState::new::<Self>(&display_handle, Vec::new()),
            _single_pixel_buffer_state: SinglePixelBufferState::new::<Self>(&display_handle),
            _output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&display_handle),
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            popups: PopupManager::default(),
            hidden_fullscreen_layers: Vec::new(),
            layer_keyboard_interactivity: HashMap::new(),
            idle_inhibiting_surfaces: Vec::new(),
            seat,
            world: World::new(Camera::new(GridPoint::new(0, 0), config.config().viewport)),
            managed_windows: Vec::new(),
            destroyed_window_transitions: Vec::new(),
            outputs: BTreeMap::new(),
            output_size: None,
            config,
            config_error: None,
            pending_camera_drag: None,
            pending_camera_zoom_scroll: 0.0,
            pending_window_drag: None,
            pending_window_resize: None,
            pending_floating_chord: None,
            pending_edge_placement: None,
            pointer_buttons_held: 0,
            pending_close_click: None,
            closing_pointer_chord: false,
            pending_pointer_click: None,
            suppressed_window_drag_releases: 0,
            suppressed_focus_click: false,
            cursor_image_status: CursorImageStatus::default_named(),
            cursor_override: None,
            cursor_hidden_by_activity: false,
            last_pointer_activity: Instant::now(),
            dnd_icon: None,
            last_host_pointer_position: None,
            cursor_wake: CursorWakeTrail::default(),
            cursor_wake_override: None,
            pending_screencopies: Vec::new(),
            pending_image_copy_captures: Vec::new(),
            xwayland_satellite: None,
            xwayland_display: None,
            spawned_commands: Vec::new(),
            pending_startup_commands: Vec::new(),
            backend_name: "winit",
            ipc_socket_path: None,
            presentation_clock,
            presentation_sequence: 0,
            focus_indicator_window: None,
            previous_focus_indicator: None,
            focus_indicator_started_at: Instant::now(),
            last_animation_frame: Instant::now(),
        })
    }

    pub(crate) fn spawn_command(&mut self, argv: &[String]) {
        self.spawn_os_command(argv.iter().map(std::ffi::OsString::from));
    }

    pub(crate) fn effective_cursor_status(&self) -> CursorImageStatus {
        if self.cursor_hidden_by_activity {
            CursorImageStatus::Hidden
        } else {
            self.cursor_override.map_or_else(
                || self.cursor_image_status.clone(),
                CursorImageStatus::Named,
            )
        }
    }

    pub(crate) fn note_pointer_activity(&mut self) {
        self.last_pointer_activity = Instant::now();
        if self.cursor_hidden_by_activity {
            self.cursor_hidden_by_activity = false;
            self.request_redraw();
        }
    }

    pub(crate) fn hide_cursor_for_keyboard_input(&mut self) {
        if !self.cursor_hidden_by_activity {
            self.cursor_hidden_by_activity = true;
            self.request_redraw();
        }
    }

    pub(crate) fn poll_cursor_idle(&mut self, now: Instant) {
        let delay_ms = self.config.config().mouse.cursor_hide_delay_ms;
        if delay_ms == 0 || self.cursor_hidden_by_activity {
            return;
        }
        if now.saturating_duration_since(self.last_pointer_activity)
            >= std::time::Duration::from_millis(u64::from(delay_ms))
        {
            self.cursor_hidden_by_activity = true;
            self.request_redraw();
        }
    }

    fn request_redraw(&self) {
        if let Some(sender) = &self.redraw_sender {
            let _ = sender.send(());
        }
    }

    fn spawn_os_command(&mut self, argv: impl IntoIterator<Item = std::ffi::OsString>) {
        let argv = argv.into_iter().collect::<Vec<_>>();
        let Some((program, arguments)) = argv.split_first() else {
            return;
        };
        let mut command = std::process::Command::new(program);
        command
            .args(arguments)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("XDG_CURRENT_DESKTOP", "mio")
            .env("XDG_SESSION_DESKTOP", "mio")
            .env("MIO_BACKEND", self.backend_name);
        if let Some(path) = &self.ipc_socket_path {
            command.env("MIO_SOCKET", path);
        } else {
            command.env_remove("MIO_SOCKET");
        }
        if let Some(display) = &self.xwayland_display {
            command.env("DISPLAY", display);
        } else {
            command.env_remove("DISPLAY");
        }
        match command.spawn() {
            Ok(child) => self.spawned_commands.push(child),
            Err(error) => {
                warn!(%error, command = %program.to_string_lossy(), "failed to run command");
            }
        }
    }

    pub(crate) fn queue_startup_command(&mut self, argv: Vec<std::ffi::OsString>) {
        if !argv.is_empty() {
            self.pending_startup_commands.push(argv);
        }
        self.launch_pending_startup_commands_if_output_ready();
    }

    pub(crate) fn launch_pending_startup_commands_if_output_ready(&mut self) {
        if self.space.outputs().next().is_none() {
            return;
        }
        for command in std::mem::take(&mut self.pending_startup_commands) {
            self.spawn_os_command(command);
        }
    }

    pub(crate) fn poll_spawned_commands(&mut self) {
        self.spawned_commands
            .retain_mut(|child| match child.try_wait() {
                Ok(Some(_)) => false,
                Ok(None) => true,
                Err(error) => {
                    warn!(%error, "failed to poll spawned command");
                    false
                }
            });
    }

    fn init_wayland_listener(
        display: Display<Self>,
        event_loop: &mut EventLoop<CalloopData>,
    ) -> StateResult<OsString> {
        let listening_socket = ListeningSocketSource::new_auto()?;
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle.insert_source(listening_socket, |client_stream, (), data| {
            if let Err(error) = data
                .display_handle
                .insert_client(client_stream, Arc::new(MioClientState::default()))
            {
                warn!(%error, "failed to register Wayland client");
            } else if let Some(sender) = &data.state.redraw_sender {
                // A newly accepted client may already have requests queued, but adding
                // it does not necessarily make the Display source readable in this
                // calloop iteration. Wake winit so the next loop dispatches those
                // requests immediately instead of waiting for maintenance polling.
                let _ = sender.send(());
            }
        })?;

        loop_handle.insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, data| {
                // SAFETY: calloop owns this Display source for the complete event-loop
                // lifetime, so the referenced Display is not moved or dropped here.
                let result = unsafe { display.get_mut() }.dispatch_clients(&mut data.state);
                if let Err(error) = result {
                    error!(%error, "failed to dispatch Wayland clients");
                    data.state.loop_signal.stop();
                }
                Ok(PostAction::Continue)
            },
        )?;

        Ok(socket_name)
    }

    pub fn surface_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.session_locked {
            return self
                .session_lock_surfaces
                .iter()
                .find_map(|(output, surface)| {
                    let geometry = self.space.output_geometry(output)?;
                    geometry
                        .to_f64()
                        .contains(position)
                        .then(|| (surface.wl_surface().clone(), geometry.loc.to_f64()))
                });
        }
        self.layer_surface_under(position, &[WlrLayer::Overlay, WlrLayer::Top])
            .or_else(|| {
                self.space
                    .element_under(position)
                    .and_then(|(window, location)| {
                        let scale = window.scale();
                        let local =
                            (position - location.to_f64()).upscale((1.0 / scale.x, 1.0 / scale.y));
                        window.surface_under(local, WindowSurfaceType::ALL).map(
                            |(surface, offset)| {
                                let origin = scaled_surface_origin(position, local, offset, scale);
                                (surface, origin)
                            },
                        )
                    })
            })
            .or_else(|| {
                self.layer_surface_under(position, &[WlrLayer::Bottom, WlrLayer::Background])
            })
    }

    fn layer_surface_under(
        &self,
        position: Point<f64, Logical>,
        layers: &[WlrLayer],
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.outputs().find_map(|output| {
            let output_geometry = self.space.output_geometry(output)?;
            let local = position - output_geometry.loc.to_f64();
            let map = layer_map_for_output(output);
            layers.iter().find_map(|&layer_kind| {
                let layer = map.layer_under(layer_kind, local)?.clone();
                let geometry = map.layer_geometry(&layer)?;
                let layer_local = local - geometry.loc.to_f64();
                layer
                    .surface_under(layer_local, WindowSurfaceType::ALL)
                    .map(|(surface, offset)| {
                        let origin = output_geometry.loc + geometry.loc + offset;
                        (surface, origin.to_f64())
                    })
            })
        })
    }

    pub fn layer_surface_for_surface(&self, surface: &WlSurface) -> Option<LayerSurface> {
        self.space.outputs().find_map(|output| {
            layer_map_for_output(output)
                .layer_for_surface(surface, WindowSurfaceType::ALL)
                .cloned()
        })
    }

    pub fn add_toplevel(&mut self, surface: ToplevelSurface) {
        let size = self.config.config().initial_window_size;
        let previous_focus = self.world.focused();
        let camera = *self.world.camera();
        let placement_windows = self
            .world
            .windows()
            .map(|window| (window.id().get(), window.rect(), window.grid_constraint()))
            .collect::<Vec<_>>();
        let Ok(id) = self.world.place_window(size) else {
            error!("failed to place new window in Mio World");
            surface.send_close();
            return;
        };
        let default_opacity = self.config.config().appearance.opacity;
        if let Err(error) = self
            .world
            .replace_config_window_properties(id, &[WindowProperty::Opacity(default_opacity)])
        {
            warn!(%error, window = id.get(), "failed to apply default Window opacity");
        }
        if let Some(window) = self.world.window(id) {
            info!(
                window = id.get(),
                placed_rect = ?window.rect(),
                focused = ?previous_focus.map(WindowId::get),
                camera_x = camera.position().x,
                camera_y = camera.position().y,
                camera_zoom = camera.zoom(),
                viewport = ?camera.viewport_size(),
                existing = ?placement_windows,
                "placed new Window"
            );
        }
        if let Some(previous_focus) = previous_focus {
            if let Err(error) = self.world.apply(Action::FocusWindow(previous_focus)) {
                warn!(%error, "failed to preserve focus while new Window awaits its first commit");
            }
        }
        let fallback = previous_focus.into_iter().collect::<Vec<_>>();
        let return_focus_candidates = self.return_focus_candidates(&surface, &fallback);
        let protocol_parent = surface
            .parent()
            .and_then(|parent| self.window_id_for_surface(&parent));
        let window = RenderWindow::new(Window::new_wayland_window(surface));
        let transition_enabled = self.config.config().effects.window_transition
            != WindowTransitionEffect::None
            && self.config.config().animation_speed > 0.0;
        let initial_transition = if transition_enabled { 0.0 } else { 1.0 };
        window.set_transition_progress(initial_transition, i32::from(transition_enabled));
        self.managed_windows.push(ManagedWindow {
            id,
            window: window.clone(),
            return_focus_candidates,
            protocol_parent,
            auto_floating: false,
            preserve_committed_size: false,
            ready_to_present: false,
            transition: if transition_enabled {
                WindowTransition::Opening
            } else {
                WindowTransition::Stable
            },
            snapshot_dirty: true,
            initial_commit_count: 0,
            render_rect: None,
            virtual_render_rects: BTreeMap::new(),
            render_opacity: AnimatedValue::new(1.0),
            transition_progress: {
                let mut progress = AnimatedValue::new(f64::from(initial_transition));
                progress.set_target(1.0);
                progress
            },
            opening_delay: if transition_enabled {
                OPENING_CONTENT_GRACE_SECONDS
            } else {
                0.0
            },
            client_width: AnimatedValue::new(1.0),
            client_height: AnimatedValue::new(1.0),
            last_configured_size: None,
        });
        // The first xdg configure must contain a concrete size. Activating first
        // would emit an empty configure through Space before sync_layout records it.
        self.sync_layout(true);
    }

    pub(crate) fn present_toplevel(&mut self, id: WindowId) {
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return;
        };
        if managed.ready_to_present {
            return;
        }
        managed.ready_to_present = true;
        let window = managed.window.clone();
        self.space.map_element(window, (0, 0), true);
        self.activate_window_with_camera_policy(
            id,
            SERIAL_COUNTER.next_serial(),
            CameraFollowPolicy::Always,
            None,
        );
    }

    pub(crate) fn should_defer_unclassified_toplevel(&mut self, id: WindowId) -> bool {
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return false;
        };
        managed.initial_commit_count = managed.initial_commit_count.saturating_add(1);
        !managed.ready_to_present
            && !managed.auto_floating
            && !managed.return_focus_candidates.is_empty()
            && managed.initial_commit_count == 1
    }

    pub(crate) fn focus_pending_toplevel(&mut self, id: WindowId) {
        if let Err(error) = self.world.apply(Action::FocusWindow(id)) {
            warn!(%error, "failed to focus pending new Window");
            return;
        }
        let Some(toplevel) = self
            .managed_window(id)
            .and_then(|managed| managed.window.toplevel().cloned())
        else {
            return;
        };
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });
        toplevel.send_pending_configure();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(toplevel.wl_surface().clone()),
                SERIAL_COUNTER.next_serial(),
            );
        }
    }

    pub fn remove_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(index) = self.managed_windows.iter().position(|managed| {
            managed
                .window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface.wl_surface())
        }) else {
            return;
        };
        let managed = self.managed_windows.remove(index);
        let self_destroyed = managed.transition != WindowTransition::Closing;
        let effects = self.config.config().effects;
        let animation_speed = self.config.config().animation_speed;
        if self_destroyed
            && animation_speed > 0.0
            && effects.window_transition != WindowTransitionEffect::None
        {
            let seconds = f64::from(effects.window_transition_duration) / 1000.0 / animation_speed;
            self.destroyed_window_transitions
                .push(DestroyedWindowTransition {
                    id: managed.id,
                    effect: effects.window_transition,
                    duration: Duration::from_secs_f64(seconds),
                });
        }
        let preferred_focus = if managed.protocol_parent.is_some() {
            managed.return_focus_candidates
        } else {
            Vec::new()
        };
        let removed_was_focused = window_removal_changes_focus(managed.id, self.world.focused());
        self.space.unmap_elem(&managed.window);
        if let Err(error) = self.world.remove_window(managed.id) {
            warn!(%error, "failed to remove destroyed toplevel from Mio World");
        }
        if removed_was_focused {
            restore_preferred_focus(&mut self.world, &preferred_focus);
            if let Some(focused) = self.world.focused() {
                self.restore_window_focus(focused, SERIAL_COUNTER.next_serial());
                return;
            }
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
            }
        }
        self.sync_layout(false);
    }

    pub(crate) fn begin_close_transition(&mut self, id: WindowId) {
        let Some(index) = self.managed_windows.iter().position(|managed| {
            managed.id == id && managed.transition != WindowTransition::Closing
        }) else {
            return;
        };
        if self.config.config().animation_speed == 0.0
            || self.config.config().effects.window_transition == WindowTransitionEffect::None
        {
            if let Some(toplevel) = self.managed_windows[index].window.toplevel() {
                toplevel.send_close();
            }
            return;
        }
        let preferred_focus = if self.managed_windows[index].protocol_parent.is_some() {
            self.managed_windows[index].return_focus_candidates.clone()
        } else {
            Vec::new()
        };
        self.managed_windows[index].transition = WindowTransition::Closing;
        self.managed_windows[index]
            .transition_progress
            .set_target(0.0);
        if let Err(error) = self.world.remove_window(id) {
            warn!(%error, "failed to remove closing Window from Mio World");
        }
        restore_preferred_focus(&mut self.world, &preferred_focus);
        if let Some(focused) = self.world.focused() {
            self.restore_window_focus(focused, SERIAL_COUNTER.next_serial());
        } else if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.sync_layout(false);
    }

    pub(crate) fn update_toplevel_parent(&mut self, surface: &ToplevelSurface) {
        let Some(index) = self.managed_windows.iter().position(|managed| {
            managed
                .window
                .toplevel()
                .is_some_and(|candidate| candidate.wl_surface() == surface.wl_surface())
        }) else {
            return;
        };
        let fallback = self.managed_windows[index].return_focus_candidates.clone();
        self.managed_windows[index].protocol_parent = surface
            .parent()
            .and_then(|parent| self.window_id_for_surface(&parent));
        self.managed_windows[index].return_focus_candidates =
            self.return_focus_candidates(surface, &fallback);
    }

    fn return_focus_candidates(
        &self,
        surface: &ToplevelSurface,
        fallback: &[WindowId],
    ) -> Vec<WindowId> {
        let direct_parent = surface
            .parent()
            .and_then(|parent| self.window_id_for_surface(&parent));
        let inherited = direct_parent
            .and_then(|parent| self.managed_window(parent))
            .map_or(&[][..], |managed| {
                managed.return_focus_candidates.as_slice()
            });
        merge_focus_candidates(direct_parent, inherited, fallback)
    }

    pub fn window_id_for_surface(&self, surface: &WlSurface) -> Option<WindowId> {
        self.managed_windows.iter().find_map(|managed| {
            let mut owns_surface = false;
            managed.window.with_surfaces(|candidate, _| {
                owns_surface |= candidate == surface;
            });
            owns_surface.then_some(managed.id)
        })
    }

    pub fn managed_window(&self, id: WindowId) -> Option<&ManagedWindow> {
        self.managed_windows.iter().find(|managed| managed.id == id)
    }

    pub(crate) fn automatic_floating_anchor(&self, id: WindowId) -> Option<WorldPoint> {
        self.managed_window(id)?
            .return_focus_candidates
            .iter()
            .find_map(|candidate| {
                self.world
                    .window(*candidate)
                    .map(|window| window.rect().origin())
            })
    }

    pub(crate) fn reset_window_presentation(&mut self, id: WindowId) {
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return;
        };
        managed.render_rect = None;
        managed.virtual_render_rects.clear();
    }

    pub(crate) fn reset_all_window_presentations(&mut self) {
        for managed in &mut self.managed_windows {
            managed.render_rect = None;
            managed.virtual_render_rects.clear();
        }
    }

    pub(crate) fn set_interactive_resize_preview(&mut self, id: WindowId, preview: ScreenRect) {
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return;
        };
        managed
            .render_rect
            .get_or_insert_with(|| AnimatedRect::new(preview))
            .set_current(preview);
        managed.client_width = AnimatedValue::new(f64::from(preview.width));
        managed.client_height = AnimatedValue::new(f64::from(preview.height));
        let size = (preview.width.max(1), preview.height.max(1));
        managed.last_configured_size = Some(size);
        if let Some(toplevel) = managed.window.toplevel() {
            toplevel.with_pending_state(|state| state.size = Some(size.into()));
            toplevel.send_pending_configure();
        }
    }

    pub(crate) fn set_interactive_move_preview(&mut self, id: WindowId, preview: ScreenRect) {
        let Some(managed) = self
            .managed_windows
            .iter_mut()
            .find(|managed| managed.id == id)
        else {
            return;
        };
        managed
            .render_rect
            .get_or_insert_with(|| AnimatedRect::new(preview))
            .set_current(preview);
    }

    pub fn activate_window(&mut self, id: WindowId, serial: Serial) {
        self.activate_window_with_camera_policy(
            id,
            serial,
            CameraFollowPolicy::OnFocusChange,
            None,
        );
    }

    pub(crate) fn activate_window_after_focus_change(
        &mut self,
        id: WindowId,
        previous_focus: Option<WindowId>,
        serial: Serial,
    ) {
        self.activate_window_with_camera_policy(
            id,
            serial,
            CameraFollowPolicy::Always,
            previous_focus,
        );
    }

    pub(crate) fn activate_window_without_camera(&mut self, id: WindowId, serial: Serial) {
        self.activate_window_with_camera_policy(id, serial, CameraFollowPolicy::Never, None);
    }

    fn activate_window_with_camera_policy(
        &mut self,
        id: WindowId,
        serial: Serial,
        camera_policy: CameraFollowPolicy,
        prior_logical_focus: Option<WindowId>,
    ) {
        let Some(window) = self
            .managed_window(id)
            .map(|managed| managed.window.clone())
        else {
            return;
        };
        let protocol_previous_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|surface| self.window_id_for_surface(&surface));
        let previous_focus = protocol_previous_focus
            .or(prior_logical_focus)
            .or_else(|| self.world.focused());
        if let (Some(keyboard), Some(toplevel)) = (self.seat.get_keyboard(), window.toplevel()) {
            let target = toplevel.wl_surface().clone();
            keyboard.set_focus(self, Some(target.clone()), serial);
            let actual = keyboard.current_focus();
            if actual.as_ref() != Some(&target) {
                // Popup keyboard grabs intentionally reject focus changes outside
                // their popup chain. Reset Smithay's pending focus as well as Mio's
                // logical focus so the glow and key target cannot diverge.
                keyboard.set_focus(self, actual.clone(), serial);
                let actual_id = actual
                    .as_ref()
                    .and_then(|surface| self.window_id_for_surface(surface));
                if let Some(actual_id) = actual_id {
                    if let Err(error) = self.world.apply(mio_core::Action::FocusWindow(actual_id)) {
                        warn!(%error, "failed to restore Mio focus after protocol focus rejection");
                    }
                } else if let Some(previous_focus) = prior_logical_focus {
                    if let Err(error) = self
                        .world
                        .apply(mio_core::Action::FocusWindow(previous_focus))
                    {
                        warn!(%error, "failed to restore prior Mio focus after protocol focus rejection");
                    }
                }
                debug!(
                    requested_window = id.get(),
                    actual_window = ?actual_id.map(WindowId::get),
                    "protocol keyboard grab rejected Window focus change"
                );
                return;
            }
        }
        if let Err(error) = self.world.apply(mio_core::Action::FocusWindow(id)) {
            warn!(%error, "failed to focus Mio window");
            return;
        }
        // Reasserting protocol focus must not undo an explicit CameraStep,
        // CameraNudge, or CameraPan. Camera following belongs to a semantic
        // focus change, not to repeated activation of the same Window.
        if camera_should_follow_on_activation(previous_focus, id, camera_policy) {
            if let Err(error) = self.world.apply(mio_core::Action::CameraFollow(id)) {
                warn!(%error, "failed to reveal focused Window with Camera");
            }
        }
        self.space.raise_element(&window, true);
        for managed in &self.managed_windows {
            if let Some(toplevel) = managed.window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        self.sync_layout(false);
    }

    fn restore_window_focus(&mut self, id: WindowId, serial: Serial) {
        if let Err(error) = self.world.apply(mio_core::Action::FocusWindow(id)) {
            warn!(%error, "failed to restore focus to dialog parent");
            return;
        }
        let Some(window) = self
            .managed_window(id)
            .map(|managed| managed.window.clone())
        else {
            return;
        };
        self.space.raise_element(&window, true);
        if let (Some(keyboard), Some(toplevel)) = (self.seat.get_keyboard(), window.toplevel()) {
            keyboard.set_focus(self, Some(toplevel.wl_surface().clone()), serial);
        }
        for managed in &self.managed_windows {
            if let Some(toplevel) = managed.window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        self.sync_layout(false);
    }

    pub fn set_output_size(&mut self, width: i32, height: i32) {
        self.output_size = Some((width, height));
        self.sync_layout(true);
    }

    pub(crate) fn register_output(&mut self, id: OutputId, output: smithay::output::Output) {
        self.outputs.insert(id, output);
    }

    pub(crate) fn apply_output_scale(&mut self) {
        let scale = self.config.config().output.scale;
        let mut x = 0;
        let outputs = self.outputs.values().cloned().collect::<Vec<_>>();
        let mut height = 0;
        for output in outputs {
            output.change_current_state(
                None,
                None,
                Some(smithay::output::Scale::Fractional(scale)),
                Some((x, 0).into()),
            );
            self.space.map_output(&output, (x, 0));
            layer_map_for_output(&output).arrange();
            if let Some(geometry) = self.space.output_geometry(&output) {
                x += geometry.size.w;
                height = height.max(geometry.size.h);
            }
        }
        self.space.refresh();
        if x > 0 && height > 0 {
            self.set_output_size(x, height);
        }
    }

    pub fn sync_layout(&mut self, configure_sizes: bool) {
        self.refresh_layer_visibility();
        let Some(output_size) = self.output_size else {
            return;
        };
        let camera = *self.world.camera();
        let normal_camera = normal_zoom(camera);
        let gaps = self.config.config().appearance.gaps;
        let layout = self
            .managed_windows
            .iter()
            .filter_map(|managed| {
                let rect = self.world.window(managed.id)?.rect();
                let presentation = self.world.window(managed.id)?.presentation();
                let area = self.output_render_area_for(
                    self.world.active_output(),
                    presentation,
                    output_size,
                );
                let screen = presentation_screen_rect(rect, camera, area, presentation)
                    .map(|rect| apply_window_gaps(rect, presentation, gaps, camera.zoom()));
                let client_screen =
                    presentation_screen_rect(rect, normal_camera, area, presentation).map(|rect| {
                        apply_window_gaps(rect, presentation, gaps, normal_camera.zoom())
                    });
                Some((
                    managed.id,
                    managed.window.clone(),
                    screen,
                    client_screen,
                    area.size,
                    presentation,
                ))
            })
            .collect::<Vec<_>>();

        for (id, window, screen, client_screen, bounds, presentation) in layout {
            let Some(screen) = screen else {
                self.space.unmap_elem(&window);
                continue;
            };
            let Some(client_screen) = client_screen else {
                self.space.unmap_elem(&window);
                continue;
            };
            let mut initial_configure_size = None;
            if let Some(managed) = self
                .managed_windows
                .iter_mut()
                .find(|managed| managed.id == id)
            {
                if let Some(window) = self.world.window(id) {
                    managed
                        .window
                        .set_floating(window.effective_properties().floating);
                    managed.window.set_blur(window.effective_properties().blur);
                    managed.window.set_presentation(presentation);
                    managed
                        .render_opacity
                        .set_target(f64::from(window.effective_properties().opacity));
                }
                managed
                    .render_rect
                    .get_or_insert_with(|| AnimatedRect::new(screen));
                if let Some(render_rect) = &mut managed.render_rect {
                    render_rect.set_target(screen);
                }
                let desired_size = (client_screen.width, client_screen.height);
                if let Some(size) =
                    record_initial_client_size(&mut managed.last_configured_size, desired_size)
                {
                    managed.client_width = AnimatedValue::new(f64::from(client_screen.width));
                    managed.client_height = AnimatedValue::new(f64::from(client_screen.height));
                    initial_configure_size = Some(size);
                } else {
                    managed
                        .client_width
                        .set_target(f64::from(client_screen.width));
                    managed
                        .client_height
                        .set_target(f64::from(client_screen.height));
                }
            }
            if let Some(toplevel) = window.toplevel() {
                if let Some(size) = initial_configure_size {
                    toplevel.with_pending_state(|state| state.size = Some(size.into()));
                }
                if configure_sizes {
                    configure_toplevel_presentation(toplevel, bounds, presentation);
                } else if initial_configure_size.is_some() {
                    toplevel.send_pending_configure();
                }
            }
        }
        self.sync_virtual_render_rects(output_size);
        self.raise_focused_window();
    }

    fn sync_virtual_render_rects(&mut self, output_size: (i32, i32)) {
        let gaps = self.config.config().appearance.gaps;
        let cameras = self
            .world
            .output_cameras()
            .map(|(id, camera)| (id, *camera))
            .collect::<Vec<_>>();
        let mut targets = Vec::new();
        for managed in &self.managed_windows {
            let Some(window) = self.world.window(managed.id) else {
                continue;
            };
            for &(output_id, camera) in &cameras {
                let area =
                    self.output_render_area_for(output_id, window.presentation(), output_size);
                if let Some(screen) =
                    presentation_screen_rect(window.rect(), camera, area, window.presentation())
                {
                    let screen =
                        apply_window_gaps(screen, window.presentation(), gaps, camera.zoom());
                    targets.push((managed.id, output_id, screen));
                }
            }
        }

        for (window_id, output_id, target) in targets {
            let Some(managed) = self
                .managed_windows
                .iter_mut()
                .find(|managed| managed.id == window_id)
            else {
                continue;
            };
            managed
                .virtual_render_rects
                .entry(output_id)
                .and_modify(|rect| rect.set_target(target))
                .or_insert_with(|| AnimatedRect::new(target));
        }
    }

    fn raise_focused_window(&mut self) {
        let focused = self
            .world
            .focused()
            .and_then(|id| self.managed_window(id))
            .map(|managed| managed.window.clone());
        if let Some(focused) = focused {
            self.space.raise_element(&focused, false);
        }
    }

    fn refresh_layer_visibility(&mut self) {
        let fullscreen = self
            .world
            .windows()
            .any(|window| window.presentation() == Presentation::Fullscreen);
        if fullscreen {
            let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
            for output in outputs {
                let layers = layer_map_for_output(&output)
                    .layers_on(WlrLayer::Top)
                    .cloned()
                    .collect::<Vec<_>>();
                for layer in layers {
                    layer_map_for_output(&output).unmap_layer(&layer);
                    self.hidden_fullscreen_layers.push((output.clone(), layer));
                }
            }
            return;
        }

        for (output, layer) in self.hidden_fullscreen_layers.drain(..) {
            let mut map = layer_map_for_output(&output);
            if let Err(error) = map.map_layer(&layer) {
                warn!(%error, "failed to restore layer after fullscreen");
            }
            map.arrange();
            layer.layer_surface().send_pending_configure();
        }
    }

    fn output_render_area(
        &self,
        presentation: Presentation,
        fallback_size: (i32, i32),
    ) -> Rectangle<i32, Logical> {
        let Some(output) = self.space.outputs().next() else {
            return Rectangle::new((0, 0).into(), fallback_size.into());
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return Rectangle::new((0, 0).into(), fallback_size.into());
        };
        if presentation == Presentation::Fullscreen {
            return output_geometry;
        }
        let mut area = layer_map_for_output(output).non_exclusive_zone();
        area.loc += output_geometry.loc;
        area
    }

    fn output_render_area_for(
        &self,
        id: OutputId,
        presentation: Presentation,
        fallback_size: (i32, i32),
    ) -> Rectangle<i32, Logical> {
        let Some(output) = self.outputs.get(&id) else {
            let area = self.output_render_area(presentation, fallback_size);
            return split_output_area(area, self.world.output_cameras().map(|(id, _)| id), id)
                .unwrap_or(area);
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return Rectangle::new((0, 0).into(), fallback_size.into());
        };
        if presentation == Presentation::Fullscreen {
            return output_geometry;
        }
        let mut area = layer_map_for_output(output).non_exclusive_zone();
        area.loc += output_geometry.loc;
        area
    }

    pub(crate) fn virtual_output_count(&self) -> usize {
        self.world.output_cameras().count()
    }

    pub(crate) fn virtual_window_views(&self, fallback_size: (i32, i32)) -> Vec<VirtualWindowView> {
        let cameras = self
            .world
            .output_cameras()
            .map(|(id, camera)| (id, *camera))
            .collect::<Vec<_>>();
        let mut views = Vec::new();
        for (id, _) in cameras {
            for window in self.space.elements() {
                let Some(managed) = self
                    .managed_windows
                    .iter()
                    .find(|managed| managed.window == *window)
                else {
                    continue;
                };
                let presentation = self
                    .world
                    .window(managed.id)
                    .map_or(Presentation::Normal, mio_core::Window::presentation);
                let area = self.output_render_area_for(id, presentation, fallback_size);
                let crop = area.to_physical_precise_round(1.0);
                let Some(screen) = managed
                    .virtual_render_rects
                    .get(&id)
                    .map(|rect| (*rect).current())
                else {
                    continue;
                };
                views.push(VirtualWindowView {
                    window: window.clone(),
                    screen,
                    crop,
                });
            }
        }
        views
    }

    pub(crate) fn activate_virtual_output_at(&mut self, position: Point<f64, Logical>) {
        let target = self.outputs.iter().find_map(|(&id, output)| {
            self.space
                .output_geometry(output)
                .filter(|area| area.to_f64().contains(position))
                .map(|_| id)
        });
        if let Some(target) = target {
            if self.world.active_output() != target
                && self
                    .world
                    .apply(mio_core::Action::ActivateOutput(target))
                    .is_ok()
            {
                self.sync_layout(false);
            }
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
    pub fn advance_animations(&mut self, now: Instant) -> bool {
        let pending_zoom_scroll = std::mem::take(&mut self.pending_camera_zoom_scroll);
        if pending_zoom_scroll != 0.0 {
            let current = self.world.camera().zoom();
            let target = crate::input::camera_zoom_from_scroll(current, pending_zoom_scroll);
            match self.world.apply(Action::CameraZoom(target)) {
                Ok(_) => {
                    tracing::debug!(
                        current,
                        target,
                        vertical = pending_zoom_scroll,
                        "mouse Camera zoom"
                    );
                    self.sync_layout(false);
                }
                Err(error) => tracing::debug!(%error, "mouse Camera zoom rejected"),
            }
        }
        let seconds = now
            .saturating_duration_since(self.last_animation_frame)
            .as_secs_f64()
            .min(0.1);
        self.last_animation_frame = now;
        let speed = self.config.config().animation_speed;
        let transition_speed = window_transition_speed(
            speed,
            self.config.config().effects.window_transition_duration,
        );
        let camera_zoom = self.world.camera().zoom();
        let dragged = self.pending_window_drag.map(|drag| {
            let pointer_delta = drag.current - drag.start;
            let offset =
                crate::input::continuous_drag_screen_delta(pointer_delta.x, pointer_delta.y);
            (drag.id, offset)
        });
        let resizing = self.pending_window_resize.as_ref().map(|resize| resize.id);
        let mut active = false;

        for managed in &mut self.managed_windows {
            if !managed.ready_to_present {
                continue;
            }
            for render_rect in managed.virtual_render_rects.values_mut() {
                active |= render_rect.advance(seconds, speed);
            }
            let Some(render_rect) = &mut managed.render_rect else {
                continue;
            };
            let previous_rect = (*render_rect).current();
            let rect_animation_active = render_rect.advance(seconds, speed);
            active |= managed.render_opacity.advance(seconds, speed);
            if managed.transition == WindowTransition::Opening
                && managed.initial_commit_count < OPENING_CONTENT_COMMIT_COUNT
                && managed.opening_delay > 0.0
            {
                managed.opening_delay = (managed.opening_delay - seconds).max(0.0);
                active = true;
            } else {
                active |= managed
                    .transition_progress
                    .advance(seconds, transition_speed);
            }
            #[allow(clippy::cast_possible_truncation)]
            let transition_direction = match managed.transition {
                WindowTransition::Opening => 1,
                WindowTransition::Stable => 0,
                WindowTransition::Closing => -1,
            };
            managed.window.set_transition_progress(
                managed.transition_progress.current() as f32,
                transition_direction,
            );
            if managed.transition == WindowTransition::Opening
                && managed.transition_progress.current() >= 1.0
            {
                managed.transition = WindowTransition::Stable;
            }
            active |= managed.client_width.advance(seconds, speed);
            active |= managed.client_height.advance(seconds, speed);
            let current: ScreenRect = render_rect.current();
            #[allow(clippy::cast_possible_truncation)]
            let opacity = managed.render_opacity.current() as f32;
            managed.window.set_opacity(opacity);
            let client_width = round_animation_size(managed.client_width.current());
            let client_height = round_animation_size(managed.client_height.current());
            let committed_size = SpaceElement::geometry(&managed.window.window).size;
            let mut scale = render_scale_for_committed_size(
                (current.width, current.height),
                (committed_size.w, committed_size.h),
                (client_width, client_height),
                managed.preserve_committed_size,
                camera_zoom,
            );
            if resizing == Some(managed.id) {
                scale = undistorted_resize_scale(scale);
            }
            managed.window.set_scale(scale);
            #[allow(clippy::cast_possible_truncation)]
            let displayed_width = (f64::from(committed_size.w.max(0)) * scale.x).round() as i32;
            #[allow(clippy::cast_possible_truncation)]
            let displayed_height = (f64::from(committed_size.h.max(0)) * scale.y).round() as i32;
            let mut location = (
                current.x + (current.width - displayed_width).max(0) / 2,
                current.y + (current.height - displayed_height).max(0) / 2,
            );
            if let Some((_, offset)) = dragged.filter(|(id, _)| *id == managed.id) {
                location.0 = location.0.saturating_add(offset.0);
                location.1 = location.1.saturating_add(offset.1);
            }
            self.space
                .map_element(managed.window.clone(), location, false);
            if managed.auto_floating && (rect_animation_active || previous_rect != current) {
                tracing::debug!(
                    window_id = ?managed.id,
                    previous = ?previous_rect,
                    current = ?current,
                    target = ?(*render_rect).target(),
                    committed_width = committed_size.w,
                    committed_height = committed_size.h,
                    scale_x = scale.x,
                    scale_y = scale.y,
                    location_x = location.0,
                    location_y = location.1,
                    camera_zoom,
                    "automatic floating presentation changed"
                );
            }

            let size = (client_width, client_height);
            if managed.last_configured_size != Some(size) {
                if let Some(toplevel) = managed.window.toplevel() {
                    toplevel.with_pending_state(|state| state.size = Some(size.into()));
                    toplevel.send_pending_configure();
                }
                managed.last_configured_size = Some(size);
            }
        }

        // `Space::map_element` removes and reinserts an already mapped element.
        // Updating every animated location above therefore rebuilds the order
        // within each z-index from `managed_windows` iteration order. Reapply
        // Mio's focus-derived raise after those presentation-only remaps so
        // rendering and `element_under` agree on the focused topmost Window.
        self.raise_focused_window();

        self.finish_close_transitions();
        active
    }

    fn finish_close_transitions(&mut self) {
        let finished = self
            .managed_windows
            .iter()
            .filter(|managed| {
                managed.transition == WindowTransition::Closing
                    && managed.transition_progress.current() <= 0.0
            })
            .map(|managed| managed.id)
            .collect::<Vec<_>>();
        for id in finished {
            if let Some(index) = self
                .managed_windows
                .iter()
                .position(|managed| managed.id == id)
            {
                let managed = self.managed_windows.remove(index);
                self.space.unmap_elem(&managed.window);
                if let Some(toplevel) = managed.window.toplevel() {
                    toplevel.send_close();
                }
            }
        }
    }

    pub fn sync_focus_indicator_contexts(
        &mut self,
        program: Option<&smithay::backend::renderer::gles::GlesPixelProgram>,
    ) -> bool {
        let appearance = self.config.config().appearance;
        let now = Instant::now();
        let focused = self.world.focused();
        if self.focus_indicator_window != focused {
            self.previous_focus_indicator =
                self.focus_indicator_window.map(|previous| (previous, now));
            self.focus_indicator_window = focused;
            self.focus_indicator_started_at = now;
        }
        let entering = focus_indicator_reveal(
            now.saturating_duration_since(self.focus_indicator_started_at),
            self.config.config().animation_speed,
        );
        let indicator_width = i32::try_from(appearance.focus_indicator_width).unwrap_or(i32::MAX);
        let indicator_height = i32::try_from(appearance.focus_indicator_height).unwrap_or(i32::MAX);
        let previous = self.previous_focus_indicator.and_then(|(id, started)| {
            let progress = focus_indicator_reveal(
                now.saturating_duration_since(started),
                self.config.config().animation_speed,
            );
            (progress < 1.0).then_some((id, progress))
        });
        if previous.is_none() {
            self.previous_focus_indicator = None;
        }
        let enabled = appearance.focus_indicator_width > 0
            && appearance.focus_indicator_height > 0
            && program.is_some();
        for managed in &self.managed_windows {
            let options = if enabled
                && focused == Some(managed.id)
                && managed.transition_progress.current() >= 0.95
            {
                Some(FocusGlowOptions {
                    width: indicator_width,
                    height: indicator_height,
                    color: appearance.focus_indicator_color,
                    reveal: entering,
                })
            } else if enabled && previous.is_some_and(|(id, _)| id == managed.id) {
                let progress = previous.map_or(1.0, |(_, progress)| progress);
                let mut color = appearance.focus_indicator_color;
                color[3] *= 1.0 - progress;
                Some(FocusGlowOptions {
                    width: indicator_width,
                    height: indicator_height,
                    color,
                    reveal: 1.0 - progress,
                })
            } else {
                None
            };
            managed.window.set_focus_glow_context(program, options);
        }
        enabled && (entering < 1.0 || previous.is_some())
    }
}

#[allow(clippy::cast_possible_truncation)] // Final normalized value is a shader f32.
fn focus_indicator_reveal(elapsed: Duration, animation_speed: f64) -> f32 {
    if animation_speed <= 0.0 {
        return 1.0;
    }
    let progress = (elapsed.as_secs_f64() * animation_speed / 0.26).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - progress).powi(3);
    eased as f32
}

fn world_to_output_area(
    rect: mio_core::WorldRect,
    camera: Camera,
    area: Rectangle<i32, Logical>,
) -> Option<ScreenRect> {
    let mut screen = world_to_screen(rect, camera, (area.size.w, area.size.h))?;
    screen.x = screen.x.saturating_add(area.loc.x);
    screen.y = screen.y.saturating_add(area.loc.y);
    Some(screen)
}

fn presentation_screen_rect(
    rect: mio_core::WorldRect,
    camera: Camera,
    area: Rectangle<i32, Logical>,
    presentation: Presentation,
) -> Option<ScreenRect> {
    if presentation == Presentation::Normal {
        world_to_output_area(rect, camera, area)
    } else {
        Some(ScreenRect {
            x: area.loc.x,
            y: area.loc.y,
            width: area.size.w,
            height: area.size.h,
        })
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn apply_window_gaps(
    rect: ScreenRect,
    presentation: Presentation,
    gaps: u32,
    camera_zoom: f64,
) -> ScreenRect {
    if presentation == Presentation::Normal {
        let scaled = (f64::from(gaps) * camera_zoom).round().max(0.0) as u32;
        inset_screen_rect(rect, scaled)
    } else {
        rect
    }
}

fn configure_toplevel_presentation(
    toplevel: &ToplevelSurface,
    bounds: smithay::utils::Size<i32, Logical>,
    presentation: Presentation,
) {
    toplevel.with_pending_state(|state| {
        state.bounds = Some(bounds);
        match presentation {
            Presentation::Normal => {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.states.unset(xdg_toplevel::State::Maximized);
            }
            Presentation::Maximized => {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.states.set(xdg_toplevel::State::Maximized);
            }
            Presentation::Fullscreen => {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.states.set(xdg_toplevel::State::Fullscreen);
            }
        }
    });
    toplevel.send_pending_configure();
}

fn record_initial_client_size(
    last_configured_size: &mut Option<(i32, i32)>,
    desired_size: (i32, i32),
) -> Option<(i32, i32)> {
    if last_configured_size.is_some() {
        return None;
    }
    *last_configured_size = Some(desired_size);
    Some(desired_size)
}

fn render_scale_for_committed_size(
    render_size: (i32, i32),
    committed_size: (i32, i32),
    configured_size: (i32, i32),
    preserve_committed_size: bool,
    camera_zoom: f64,
) -> Scale<f64> {
    if preserve_committed_size && committed_size.0 > 0 && committed_size.1 > 0 {
        return Scale::from(camera_zoom);
    }
    let width = if committed_size.0 > 0 {
        committed_size.0
    } else {
        configured_size.0.max(1)
    };
    let height = if committed_size.1 > 0 {
        committed_size.1
    } else {
        configured_size.1.max(1)
    };
    Scale::from((
        f64::from(render_size.0) / f64::from(width),
        f64::from(render_size.1) / f64::from(height),
    ))
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn presented_corner_radius(corner_radius: u32, output_scale: f64, render_scale: f64) -> f32 {
    corner_radius as f32 * output_scale as f32 * render_scale as f32
}

fn undistorted_resize_scale(scale: Scale<f64>) -> Scale<f64> {
    Scale::from(scale.x.min(scale.y))
}

fn window_removal_changes_focus(removed: WindowId, focused: Option<WindowId>) -> bool {
    focused == Some(removed)
}

fn restore_preferred_focus(world: &mut World, candidates: &[WindowId]) {
    let Some(preferred) = candidates
        .iter()
        .copied()
        .find(|id| world.window(*id).is_some())
    else {
        return;
    };
    if let Err(error) = world.apply(Action::FocusWindow(preferred)) {
        warn!(%error, "failed to restore focus to dialog parent");
    }
}

fn merge_focus_candidates(
    direct_parent: Option<WindowId>,
    inherited: &[WindowId],
    fallback: &[WindowId],
) -> Vec<WindowId> {
    let mut candidates = Vec::new();
    for id in direct_parent
        .into_iter()
        .chain(inherited.iter().copied())
        .chain(fallback.iter().copied())
    {
        if !candidates.contains(&id) {
            candidates.push(id);
        }
    }
    candidates
}

fn split_output_area(
    area: Rectangle<i32, Logical>,
    ids: impl IntoIterator<Item = OutputId>,
    target: OutputId,
) -> Option<Rectangle<i32, Logical>> {
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort_unstable();
    let index = ids.iter().position(|id| *id == target)?;
    let count = i32::try_from(ids.len()).ok()?.max(1);
    let left =
        i32::try_from(i64::from(area.size.w) * i64::try_from(index).ok()? / i64::from(count))
            .ok()?;
    let right =
        i32::try_from(i64::from(area.size.w) * i64::try_from(index + 1).ok()? / i64::from(count))
            .ok()?;
    Some(Rectangle::new(
        (area.loc.x.saturating_add(left), area.loc.y).into(),
        (right.saturating_sub(left), area.size.h).into(),
    ))
}

fn camera_should_follow_on_activation(
    previous_focus: Option<WindowId>,
    activated: WindowId,
    policy: CameraFollowPolicy,
) -> bool {
    match policy {
        CameraFollowPolicy::Always => true,
        CameraFollowPolicy::OnFocusChange => previous_focus != Some(activated),
        CameraFollowPolicy::Never => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraFollowPolicy {
    Always,
    OnFocusChange,
    Never,
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::time::Duration;

    use super::{
        apply_window_gaps, camera_should_follow_on_activation, floating_z_index,
        focus_indicator_reveal, merge_focus_candidates, presentation_screen_rect,
        presented_corner_radius, record_initial_client_size, render_scale_for_committed_size,
        restore_preferred_focus, scaled_surface_origin, should_round_surface_element,
        split_output_area, undistorted_resize_scale, window_removal_changes_focus,
        window_transition_speed, CameraFollowPolicy, ScreenRect,
    };

    #[test]
    fn rounded_window_clip_does_not_apply_to_popup_surfaces() {
        assert!(should_round_surface_element(false, true));
        assert!(!should_round_surface_element(true, true));
        assert!(!should_round_surface_element(false, false));
    }

    #[test]
    fn pointer_surface_origin_applies_the_window_presentation_scale() {
        let position = Point::<f64, Logical>::from((500.0, 300.0));
        let window_local = Point::<f64, Logical>::from((200.0, 100.0));
        let popup_offset = Point::<i32, Logical>::from((160, 80));
        assert_eq!(
            scaled_surface_origin(
                position,
                window_local,
                popup_offset,
                Scale::from((0.5, 0.5)),
            ),
            Point::from((480.0, 290.0))
        );
    }
    use mio_core::{
        Camera, GridPoint, GridRect, GridSize, OutputId, Presentation, WindowId, World,
    };
    use smithay::utils::{Logical, Point, Rectangle, Scale};

    #[test]
    fn floating_windows_stack_above_tiled_windows() {
        assert!(floating_z_index(true) > floating_z_index(false));
    }

    #[test]
    fn focus_light_reveals_from_center_and_respects_disabled_animation() {
        assert_eq!(focus_indicator_reveal(Duration::ZERO, 1.0), 0.0);
        let halfway = focus_indicator_reveal(Duration::from_millis(130), 1.0);
        assert!(halfway > 0.0 && halfway < 1.0);
        assert_eq!(focus_indicator_reveal(Duration::from_millis(260), 1.0), 1.0);
        assert_eq!(focus_indicator_reveal(Duration::ZERO, 0.0), 1.0);
    }

    #[test]
    fn transition_duration_maps_to_the_requested_settling_time() {
        let speed = window_transition_speed(1.0, 625);
        assert!((speed - 0.399_430_97).abs() < 0.000_001);
        assert!(window_transition_speed(0.0, 625).abs() < f64::EPSILON);
    }

    #[test]
    fn initial_client_size_is_configured_without_waiting_for_a_redraw() {
        let mut last = None;
        assert_eq!(
            record_initial_client_size(&mut last, (1280, 720)),
            Some((1280, 720))
        );
        assert_eq!(last, Some((1280, 720)));
        assert_eq!(record_initial_client_size(&mut last, (1920, 1080)), None);
    }

    #[test]
    fn render_scale_uses_the_size_the_client_has_actually_committed() {
        let committed =
            render_scale_for_committed_size((1600, 900), (800, 600), (1600, 900), false, 1.0);
        assert!((committed.x - 2.0).abs() < f64::EPSILON);
        assert!((committed.y - 1.5).abs() < f64::EPSILON);
        let fallback =
            render_scale_for_committed_size((1600, 900), (0, 0), (1600, 900), false, 1.0);
        assert!((fallback.x - 1.0).abs() < f64::EPSILON);
        assert!((fallback.y - 1.0).abs() < f64::EPSILON);
        let fixed = render_scale_for_committed_size((800, 450), (640, 480), (1600, 900), true, 0.5);
        assert!((fixed.x - 0.5).abs() < f64::EPSILON);
        assert!((fixed.y - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn zoom_scaled_gaps_keep_surface_tree_presentation_scale_uniform() {
        let normal = apply_window_gaps(
            ScreenRect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            Presentation::Normal,
            20,
            1.0,
        );
        let distant = apply_window_gaps(
            ScreenRect {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            },
            Presentation::Normal,
            20,
            0.5,
        );
        let scale = render_scale_for_committed_size(
            (distant.width, distant.height),
            (normal.width, normal.height),
            (normal.width, normal.height),
            false,
            0.5,
        );

        assert!((scale.x - 0.5).abs() < f64::EPSILON);
        assert!((scale.y - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn corner_radius_follows_camera_presentation_scale() {
        assert!((presented_corner_radius(16, 1.0, 0.5) - 8.0).abs() < f32::EPSILON);
        assert!((presented_corner_radius(16, 2.0, 0.5) - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn interactive_resize_preserves_the_buffer_aspect_ratio() {
        let scale = undistorted_resize_scale(Scale::from((0.5, 1.75)));
        assert!((scale.x - 0.5).abs() < f64::EPSILON);
        assert!((scale.y - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn removing_an_unfocused_window_does_not_reactivate_focus() {
        let focused = WindowId::from_u64(1);
        let removed = WindowId::from_u64(2);
        assert!(!window_removal_changes_focus(removed, Some(focused)));
        assert!(window_removal_changes_focus(focused, Some(focused)));
    }

    #[test]
    fn repeated_activation_does_not_restore_a_manually_moved_camera() {
        let focused = WindowId::from_u64(1);
        assert!(!camera_should_follow_on_activation(
            Some(focused),
            focused,
            CameraFollowPolicy::OnFocusChange
        ));
        assert!(camera_should_follow_on_activation(
            Some(focused),
            WindowId::from_u64(2),
            CameraFollowPolicy::OnFocusChange
        ));
        assert!(!camera_should_follow_on_activation(
            None,
            focused,
            CameraFollowPolicy::Never
        ));
    }

    #[test]
    fn first_presentation_follows_even_when_pending_window_already_has_focus() {
        let pending = WindowId::from_u64(2);
        assert!(camera_should_follow_on_activation(
            Some(pending),
            pending,
            CameraFollowPolicy::Always
        ));
    }

    #[test]
    fn removing_a_focused_dialog_prefers_its_surviving_parent() {
        let mut world = World::new(Camera::new(
            GridPoint::new(0, 0),
            GridSize::new(4, 4).unwrap(),
        ));
        let foot = world
            .add_window(GridRect::new(0, 0, 1, 1).unwrap())
            .unwrap();
        let parent = world
            .add_window(GridRect::new(1, 0, 1, 1).unwrap())
            .unwrap();
        let dialog = world
            .add_window(GridRect::new(2, 0, 1, 1).unwrap())
            .unwrap();
        world.focus_window(dialog).unwrap();

        world.remove_window(dialog).unwrap();
        assert_eq!(world.focused(), None);
        assert!(world.window(foot).is_some());
        restore_preferred_focus(&mut world, &[parent]);

        assert_eq!(world.focused(), Some(parent));
    }

    #[test]
    fn nested_dialog_prefers_live_parent_then_ancestor_then_focus_history() {
        let main = WindowId::from_u64(1);
        let chooser = WindowId::from_u64(2);
        let foot = WindowId::from_u64(3);
        assert_eq!(
            merge_focus_candidates(Some(chooser), &[main], &[chooser, foot]),
            vec![chooser, main, foot]
        );
        assert_eq!(merge_focus_candidates(None, &[], &[chooser]), vec![chooser]);
    }

    #[test]
    fn virtual_outputs_split_odd_width_without_gaps() {
        let area = Rectangle::new((10, 4).into(), (101, 60).into());
        let ids = [OutputId::from_raw(2), OutputId::from_raw(1)];
        let left = split_output_area(area, ids, OutputId::from_raw(1)).unwrap();
        let right = split_output_area(area, ids, OutputId::from_raw(2)).unwrap();

        assert_eq!(left, Rectangle::new((10, 4).into(), (50, 60).into()));
        assert_eq!(right, Rectangle::new((60, 4).into(), (51, 60).into()));
    }

    #[test]
    fn gaps_apply_only_to_normal_window_presentation() {
        let rect = ScreenRect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            apply_window_gaps(rect, Presentation::Normal, 8, 1.0),
            ScreenRect {
                x: 8,
                y: 8,
                width: 784,
                height: 584,
            }
        );
        assert_eq!(
            apply_window_gaps(rect, Presentation::Maximized, 8, 1.0),
            rect
        );
        assert_eq!(
            apply_window_gaps(rect, Presentation::Fullscreen, 8, 1.0),
            rect
        );
    }

    #[test]
    fn gaps_follow_camera_zoom_to_preserve_window_proportions() {
        let rect = ScreenRect {
            x: 100,
            y: 50,
            width: 200,
            height: 120,
        };

        assert_eq!(
            apply_window_gaps(rect, Presentation::Normal, 20, 0.5),
            ScreenRect {
                x: 110,
                y: 60,
                width: 180,
                height: 100,
            }
        );
    }

    #[test]
    fn presented_windows_use_the_exact_output_area_independent_of_camera() {
        let mut camera = Camera::new(GridPoint::new(0, 0), GridSize::new(8, 8).unwrap());
        camera.set_zoom(0.5).unwrap();
        camera
            .move_to(mio_core::CameraPosition::new(0.75, -1.25).unwrap())
            .unwrap();
        let rect = GridRect::new(0, -2, 8, 8).unwrap().into();
        let area = Rectangle::new((40, 20).into(), (960, 540).into());
        let expected = Some(ScreenRect {
            x: 40,
            y: 20,
            width: 960,
            height: 540,
        });

        assert_ne!(
            presentation_screen_rect(rect, camera, area, Presentation::Normal),
            expected
        );
        assert_eq!(
            presentation_screen_rect(rect, camera, area, Presentation::Maximized),
            expected
        );
        assert_eq!(
            presentation_screen_rect(rect, camera, area, Presentation::Fullscreen),
            expected
        );
        assert_eq!(camera.zoom(), 0.5);
        assert_eq!(camera.position().x, 0.75);
        assert_eq!(camera.position().y, -1.25);
    }
}

fn normal_zoom(mut camera: Camera) -> Camera {
    camera.set_zoom(1.0).expect("normal Camera zoom is valid");
    camera
}

fn round_animation_size(value: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value.round() as i32;
    rounded.max(1)
}

#[derive(Default)]
pub struct MioClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for MioClientState {
    fn initialized(&self, client_id: ClientId) {
        tracing::debug!(?client_id, "Wayland client connected");
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        match reason {
            DisconnectReason::ConnectionClosed => {
                tracing::debug!(?client_id, "Wayland client connection closed");
            }
            DisconnectReason::ProtocolError(error) => {
                tracing::warn!(
                    ?client_id,
                    ?error,
                    "Wayland client disconnected after protocol error"
                );
            }
        }
    }
}
