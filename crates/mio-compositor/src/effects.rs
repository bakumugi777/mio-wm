use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::CString,
    mem::{size_of, size_of_val},
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Element, Id, RenderElement, UnderlyingStorage},
            gles::{
                ffi, GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram,
                GlesTexture, Uniform, UniformName, UniformType,
            },
            utils::{CommitCounter, DamageSet, OpaqueRegions},
            Bind, BlitFrame, Frame, FrameContext, Offscreen, Renderer, Texture, TextureFilter,
        },
    },
    utils::{
        user_data::UserDataMap, Buffer, Logical, Physical, Point, Rectangle, Scale, Transform,
    },
};

/// Draws an element normally while preventing opaque-region culling from
/// removing the scene behind it.
#[derive(Clone, Debug)]
pub struct NonOccludingElement<E>(E);

impl<E> NonOccludingElement<E> {
    pub fn new(element: E) -> Self {
        Self(element)
    }
}

impl<E: Element> Element for NonOccludingElement<E> {
    fn id(&self) -> &Id {
        self.0.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.0.location(scale)
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.0.src()
    }
    fn transform(&self) -> Transform {
        self.0.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(scale, commit)
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.0.alpha()
    }
    fn kind(&self) -> smithay::backend::renderer::element::Kind {
        self.0.kind()
    }
    fn is_framebuffer_effect(&self) -> bool {
        self.0.is_framebuffer_effect()
    }
}

impl<R, E> RenderElement<R> for NonOccludingElement<E>
where
    R: Renderer,
    E: RenderElement<R>,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.0.draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), R::Error> {
        self.0.capture_framebuffer(frame, src, dst, cache)
    }
}

const WAKE_LIFETIME: Duration = Duration::from_millis(1400);
const WAKE_GESTURE_GAP: Duration = Duration::from_millis(200);
const WAKE_MIN_SAMPLE_DISTANCE: f64 = 6.0;
const WAKE_MAX_SAMPLES: usize = 192;

const FOCUS_GLOW_SHADER: &str = r"
//_DEFINES_
precision highp float;
uniform vec2 size;
uniform float alpha;
uniform vec4 glow_color;
uniform float line_height;
uniform float glow_width;
uniform float reveal;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    // The light core sits in the upper quarter of the element so most of the
    // available area can carry its reflection below the Window edge.
    vec2 point = v_coords - vec2(0.5, 0.25);
    float glow_half_width = max(glow_width * 0.5, 1.0);
    float horizontal_distance = abs(point.x * size.x);
    float core_horizontal = 1.0 - smoothstep(
        glow_half_width * 0.56,
        glow_half_width,
        horizontal_distance
    );
    float bloom_horizontal = 1.0 - smoothstep(
        glow_half_width * 0.72,
        glow_half_width * 1.75,
        horizontal_distance
    );
    float reveal_edge = mix(0.03, 0.5, reveal);
    float reveal_mask = 1.0 - smoothstep(max(reveal_edge - 0.08, 0.0), reveal_edge, abs(point.x));
    core_horizontal *= reveal_mask;
    bloom_horizontal *= reveal_mask;
    float distance = abs(point.y) / max(line_height, 0.001);
    float core = exp(-0.5 * distance * distance);
    // A broad, low-energy halo makes the indicator read as emitted light while
    // keeping the narrow core and the Window content legible.
    float halo = exp(-0.024 * distance * distance) * 0.24;
    float downward = smoothstep(0.0, line_height * 0.8, point.y);
    float lower_glow = exp(-0.010 * distance * distance) * 0.32 * downward;
    float membrane_edge = 1.0 - smoothstep(0.46, 0.5, abs(point.x));
    // Keep the visible film just inside the Window. The surrounding halo may cross
    // the edge, but the film itself must remain legible on a light background.
    float membrane_height = max(line_height * 2.25, 0.001);
    float membrane_distance = abs(point.y + membrane_height * 0.42) / membrane_height;
    float membrane = (1.0 - smoothstep(0.12, 0.92, membrane_distance)) * 0.82;
    membrane *= membrane_edge * reveal_mask;
    float opacity = glow_color.a * (
        core_horizontal * core
        + bloom_horizontal * (halo + lower_glow)
        + membrane
    ) * alpha;
    opacity = min(opacity, 1.0);
    vec3 color = glow_color.rgb * opacity;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec3(0.0, 0.2, 0.0) * opacity * 0.2 + color * 0.8;
#endif
    gl_FragColor = vec4(color, opacity);
}
";

pub fn compile_focus_glow_shader(
    renderer: &mut GlesRenderer,
) -> Result<GlesPixelProgram, GlesError> {
    renderer.compile_custom_pixel_shader(
        FOCUS_GLOW_SHADER,
        &[
            UniformName::new("glow_color", UniformType::_4f),
            UniformName::new("line_height", UniformType::_1f),
            UniformName::new("glow_width", UniformType::_1f),
            UniformName::new("reveal", UniformType::_1f),
        ],
    )
}

#[derive(Clone, Debug)]
pub struct FocusGlowElement {
    id: Id,
    geometry: Rectangle<i32, Logical>,
    program: GlesPixelProgram,
    color: [f32; 4],
    line_height: f32,
    glow_width: f32,
    reveal: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusGlowOptions {
    pub width: i32,
    pub height: i32,
    pub color: [f32; 4],
    pub reveal: f32,
}

impl FocusGlowElement {
    #[allow(clippy::cast_precision_loss)] // Renderer uniforms use f32 pixel measures.
    pub fn new(
        id: Id,
        window_geometry: Rectangle<i32, Logical>,
        program: GlesPixelProgram,
        options: FocusGlowOptions,
    ) -> Self {
        let geometry = focus_glow_geometry(window_geometry, options.height);
        Self {
            id,
            geometry,
            program,
            color: options.color,
            line_height: options.height as f32 / geometry.size.h as f32,
            glow_width: options.width.clamp(1, window_geometry.size.w.max(1)) as f32,
            reveal: options.reveal,
        }
    }
}

fn focus_glow_geometry(
    window_geometry: Rectangle<i32, Logical>,
    height: i32,
) -> Rectangle<i32, Logical> {
    let glow_height = height.saturating_mul(16).max(32);
    let center_y = window_geometry.loc.y.saturating_add(window_geometry.size.h);
    Rectangle::new(
        (
            window_geometry.loc.x,
            center_y.saturating_sub(glow_height / 4),
        )
            .into(),
        (window_geometry.size.w.max(1), glow_height).into(),
    )
}

impl Element for FocusGlowElement {
    fn id(&self) -> &Id {
        &self.id
    }

    #[allow(clippy::cast_sign_loss)] // Bit patterns intentionally feed an opaque commit hash.
    fn current_commit(&self) -> CommitCounter {
        let mut hash = self.geometry.loc.x as usize ^ self.geometry.loc.y as usize;
        hash = hash.rotate_left(7) ^ self.geometry.size.w as usize;
        hash = hash.rotate_left(7) ^ self.geometry.size.h as usize;
        for value in self.color {
            hash = hash.rotate_left(7) ^ value.to_bits() as usize;
        }
        hash = hash.rotate_left(7) ^ self.reveal.to_bits() as usize;
        CommitCounter::from(hash)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            self.geometry
                .size
                .to_f64()
                .to_buffer(1.0, Transform::Normal),
        )
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }
}

impl RenderElement<GlesRenderer> for FocusGlowElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_pixel_shader_to(
            &self.program,
            src,
            dst,
            self.geometry.size.to_buffer(1, Transform::Normal),
            Some(damage),
            1.0,
            &[
                Uniform::new("glow_color", self.color),
                Uniform::new("line_height", self.line_height),
                Uniform::new("glow_width", self.glow_width),
                Uniform::new("reveal", self.reveal),
            ],
        )
    }
}

const WINDOW_BORDER_SHADER: &str = r"
precision highp float;
uniform vec2 size;
uniform float alpha;
uniform float border_width;
uniform float corner_radius;
uniform vec4 border_color;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

float rounded_box_distance(vec2 point, vec2 half_size, float radius) {
    vec2 distance = abs(point) - (half_size - vec2(radius));
    return length(max(distance, 0.0))
        + min(max(distance.x, distance.y), 0.0) - radius;
}

void main() {
    vec2 point = v_coords * size;
    vec2 half_size = size * 0.5;
    float radius = min(corner_radius, min(half_size.x, half_size.y));
    float distance = rounded_box_distance(point - half_size, half_size, radius);
    float inside = 1.0 - smoothstep(-0.5, 0.5, distance);
    float inner = smoothstep(-border_width - 0.5, -border_width + 0.5, distance);
    float opacity = border_color.a * inside * inner * alpha;
    vec3 color = border_color.rgb * opacity;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec3(0.0, 0.2, 0.0) * opacity * 0.2 + color * 0.8;
#endif
    gl_FragColor = vec4(color, opacity);
}
";

pub fn compile_window_border_shader(
    renderer: &mut GlesRenderer,
) -> Result<GlesPixelProgram, GlesError> {
    renderer.compile_custom_pixel_shader(
        WINDOW_BORDER_SHADER,
        &[
            UniformName::new("border_width", UniformType::_1f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("border_color", UniformType::_4f),
        ],
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowBorderOptions {
    pub width: f32,
    pub color: [f32; 4],
    pub corner_radius: f32,
}

#[derive(Clone, Debug)]
pub struct WindowBorderElement {
    id: Id,
    geometry: Rectangle<i32, Logical>,
    program: GlesPixelProgram,
    options: WindowBorderOptions,
}

impl WindowBorderElement {
    pub fn new(
        id: Id,
        geometry: Rectangle<i32, Logical>,
        program: GlesPixelProgram,
        options: WindowBorderOptions,
    ) -> Self {
        Self {
            id,
            geometry,
            program,
            options,
        }
    }
}

impl Element for WindowBorderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        let mut hash = self.options.width.to_bits() as usize;
        hash = hash.rotate_left(7) ^ self.options.corner_radius.to_bits() as usize;
        for value in self.options.color {
            hash = hash.rotate_left(7) ^ value.to_bits() as usize;
        }
        CommitCounter::from(hash)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            self.geometry
                .size
                .to_f64()
                .to_buffer(1.0, Transform::Normal),
        )
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }
}

impl RenderElement<GlesRenderer> for WindowBorderElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_pixel_shader_to(
            &self.program,
            src,
            dst,
            self.geometry.size.to_buffer(1, Transform::Normal),
            Some(damage),
            1.0,
            &[
                Uniform::new("border_width", self.options.width),
                Uniform::new("corner_radius", self.options.corner_radius),
                Uniform::new("border_color", self.options.color),
            ],
        )
    }
}

// Archived ripple-chain implementation. Keep these shaders together with the
// restoration notes in `memo/cursor-wake-ripple-chain.md` until the ribbon wake
// has been visually accepted.
const _LEGACY_CURSOR_WAKE_SHADER: &str = r"
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
uniform vec2 screen_size;
uniform vec4 wake_0;
uniform vec4 wake_1;
uniform vec4 wake_2;
uniform vec4 wake_3;
uniform vec3 wake_state_0;
uniform vec3 wake_state_1;
uniform vec3 wake_state_2;
uniform vec3 wake_state_3;
uniform int wake_count;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

vec2 wake_displacement(vec2 point, vec4 endpoints, vec3 state, inout float lighting) {
    vec2 start = endpoints.xy;
    vec2 end = endpoints.zw;
    vec2 segment = end - start;
    float segment_length = max(length(segment), 0.0001);
    vec2 direction = segment / segment_length;
    vec2 normal = vec2(-direction.y, direction.x);
    vec2 from_end = point - end;
    float behind = clamp(-dot(from_end, direction), 0.0, segment_length);
    float along = behind / segment_length;
    float sideways = dot(from_end + direction * behind, normal);
    float trail_width = mix(9.0, 28.0, sin(along * 3.14159265));
    float spread = state.y * 42.0 + along * 16.0;
    float wave_width = mix(3.0, 6.0, state.y);
    float distance_from_wave = abs(sideways) - spread;
    float lateral = exp(-pow(distance_from_wave / wave_width, 2.0) * 1.8);
    float longitudinal = smoothstep(0.0, 0.12, along)
        * (1.0 - smoothstep(0.82, 1.0, along));
    float envelope = 1.0 - smoothstep(
        0.0,
        spread + wave_width,
        abs(sideways) - trail_width
    );
    float mask = lateral * longitudinal * envelope;
    float fade = pow(1.0 - state.y, 1.35);
    float outward = sideways < 0.0 ? -1.0 : 1.0;
    float wave = cos(distance_from_wave * 0.45) * outward;
    vec2 trail = normal * wave * mask * state.x * fade * 7.0;

    float forward = dot(from_end, direction);
    float head_distance = length(vec2(forward / 1.35, sideways));
    float head_mask = exp(-pow(head_distance / 18.0, 2.0) * 2.2) * state.z;
    vec2 bow = normal * outward * head_mask * state.x * fade * 3.2;
    lighting += outward * lateral * longitudinal * envelope * state.x * fade * 0.11;
    lighting += outward * head_mask * state.x * fade * 0.045;
    return trail + bow;
}

void main() {
    vec2 point = v_coords * screen_size;
    float lighting = 0.0;
    vec2 displacement = wake_displacement(point, wake_0, wake_state_0, lighting);
    if (wake_count > 1) displacement += wake_displacement(point, wake_1, wake_state_1, lighting);
    if (wake_count > 2) displacement += wake_displacement(point, wake_2, wake_state_2, lighting);
    if (wake_count > 3) displacement += wake_displacement(point, wake_3, wake_state_3, lighting);
    vec2 sample_coords = v_coords + displacement / screen_size;
    vec4 color = texture2D(tex, clamp(sample_coords, vec2(0.0), vec2(1.0)));
    float surface_light = clamp(lighting, -0.12, 0.12);
    color.rgb *= 1.0 + surface_light;
    color.rgb += vec3(max(surface_light, 0.0) * 0.25);
#if defined(NO_ALPHA)
    color.a = 1.0;
#endif
    color *= alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
";

const _LEGACY_WATER_UPDATE_SHADER: &str = r"
//_DEFINES_
precision highp float;
uniform sampler2D tex;
uniform float alpha;
uniform vec2 screen_size;
uniform vec4 wake_0;
uniform vec4 wake_1;
uniform vec4 wake_2;
uniform vec4 wake_3;
uniform vec4 wake_4;
uniform vec4 wake_5;
uniform vec4 wake_6;
uniform vec4 wake_7;
uniform vec4 wake_strengths_0;
uniform vec4 wake_strengths_1;
uniform int wake_count;
uniform float wake_width;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
void nearest_stroke(
    vec2 point,
    vec4 endpoints,
    float strength,
    float is_head,
    inout float nearest_distance,
    inout float nearest_signed_distance,
    inout float nearest_strength,
    inout vec2 nearest_tangent,
    inout float nearest_head_taper
) {
    if (strength <= 0.0) return;
    vec2 segment = endpoints.zw - endpoints.xy;
    float segment_length = max(length(segment), 0.0001);
    vec2 tangent = segment / segment_length;
    float raw_along = dot(point - endpoints.xy, tangent);
    if (raw_along > segment_length) return;
    float along = clamp(raw_along, 0.0, segment_length);
    vec2 offset = point - (endpoints.xy + tangent * along);
    float distance = length(offset);
    if (distance < nearest_distance) {
        nearest_distance = distance;
        nearest_signed_distance = tangent.x * offset.y - tangent.y * offset.x;
        nearest_strength = strength;
        nearest_tangent = tangent;
        float taper_length = min(max(segment_length, 1.0), 36.0);
        nearest_head_taper = is_head > 0.5
            ? smoothstep(0.0, taper_length, segment_length - along)
            : 1.0;
    }
}
void main() {
    vec2 texel = 4.0 / screen_size;
    float height = texture2D(tex, v_coords).r * 2.0 - 1.0;
    float velocity = texture2D(tex, v_coords).g * 2.0 - 1.0;
    float left = texture2D(tex, v_coords - vec2(texel.x, 0.0)).r * 2.0 - 1.0;
    float right = texture2D(tex, v_coords + vec2(texel.x, 0.0)).r * 2.0 - 1.0;
    float up = texture2D(tex, v_coords - vec2(0.0, texel.y)).r * 2.0 - 1.0;
    float down = texture2D(tex, v_coords + vec2(0.0, texel.y)).r * 2.0 - 1.0;
    float laplacian = left + right + up + down - 4.0 * height;
    vec2 point = v_coords * screen_size;
    float force = 0.0;
    float distance = 1000000.0;
    float signed_distance = 0.0;
    float strength = 0.0;
    vec2 tangent = vec2(1.0, 0.0);
    float taper = 1.0;
    nearest_stroke(point, wake_0, wake_strengths_0.x, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 1) nearest_stroke(point, wake_1, wake_strengths_0.y, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 2) nearest_stroke(point, wake_2, wake_strengths_0.z, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 3) nearest_stroke(point, wake_3, wake_strengths_0.w, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 4) nearest_stroke(point, wake_4, wake_strengths_1.x, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 5) nearest_stroke(point, wake_5, wake_strengths_1.y, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 6) nearest_stroke(point, wake_6, wake_strengths_1.z, 0.0, distance, signed_distance, strength, tangent, taper);
    if (wake_count > 7) nearest_stroke(point, wake_7, wake_strengths_1.w, 0.0, distance, signed_distance, strength, tangent, taper);
    force += exp(-pow(distance / max(wake_width, 1.0), 2.0) * 2.0) * strength;
    velocity = (velocity + laplacian * 0.22 + force * 0.055) * 0.970;
    height = (height + velocity) * 0.985;
    gl_FragColor = vec4(clamp(height * 0.5 + 0.5, 0.0, 1.0), clamp(velocity * 0.5 + 0.5, 0.0, 1.0), 0.0, 1.0);
}
";

const _LEGACY_WATER_COMPOSITE_SHADER: &str = r"
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform sampler2D water_tex;
uniform float alpha;
uniform vec2 screen_size;
uniform vec2 water_half_pixel;
uniform float distortion_strength;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
void main() {
    vec4 ribbon = texture2D(water_tex, v_coords);
    vec2 flow = (ribbon.rg - vec2(0.5)) * 2.0;
    vec2 sample_coords = clamp(v_coords + flow * distortion_strength, vec2(0.0), vec2(1.0));
    vec4 color = texture2D(tex, sample_coords);
#if defined(NO_ALPHA)
    color.a = 1.0;
#endif
    gl_FragColor = color * alpha;
}
";

const RIBBON_VERTEX_SHADER: &str = r"#version 100
precision highp float;
attribute vec2 position;
attribute vec2 normal;
attribute float side;
attribute float opacity;
uniform vec2 screen_size;
varying vec2 v_normal;
varying float v_side;
varying float v_opacity;
varying vec2 v_uv;
void main() {
    vec2 clip = vec2(position.x / screen_size.x * 2.0 - 1.0,
                     position.y / screen_size.y * 2.0 - 1.0);
    gl_Position = vec4(clip, 0.0, 1.0);
    v_normal = normal;
    v_side = side;
    v_opacity = opacity;
    v_uv = position / screen_size;
}
";

const RIBBON_FRAGMENT_SHADER: &str = r"#version 100
precision highp float;
varying vec2 v_normal;
varying float v_side;
varying float v_opacity;
varying vec2 v_uv;
uniform sampler2D screen_tex;
uniform vec2 screen_size;
uniform float distortion_strength;
float hash1(float value) {
    return fract(sin(value * 127.1 + 311.7) * 43758.5453);
}
float value_noise(float value) {
    float cell = floor(value);
    float fraction = fract(value);
    float blend = fraction * fraction * (3.0 - 2.0 * fraction);
    return mix(hash1(cell), hash1(cell + 1.0), blend) * 2.0 - 1.0;
}
void main() {
    vec2 normal = v_normal / max(length(v_normal), 0.001);
    vec2 tangent = vec2(normal.y, -normal.x);
    float edge_distance = abs(v_side);
    float outer_fade = 1.0 - smoothstep(0.78, 1.0, edge_distance);
    float edge_emphasis = smoothstep(0.12, 0.68, edge_distance);
    float distortion_profile = mix(0.10, 1.0, edge_emphasis) * outer_fade;
    float along = dot(v_uv * screen_size, tangent) / 84.0;
    float normal_wander = value_noise(along) * 0.05;
    vec2 flow_direction = normalize(tangent + normal * normal_wander);
    vec2 offset = flow_direction * distortion_profile * v_opacity
        * (distortion_strength * 320.0) / screen_size;
    vec4 color = texture2D(screen_tex, clamp(v_uv + offset, vec2(0.0), vec2(1.0)));
    // Refraction is invisible over a perfectly flat background. Derive a small
    // directional reflection from the ribbon's actual surface normal instead
    // of applying a symmetric bright/dark stripe across the wake.
    vec3 surface_normal = normalize(vec3(normal * (-v_side) * 0.72, 1.0));
    vec3 light_direction = normalize(vec3(-0.38, -0.46, 0.80));
    float reflection = pow(max(dot(surface_normal, light_direction), 0.0), 14.0)
        * distortion_profile * v_opacity * 0.24;
    color.rgb = mix(color.rgb, vec3(0.92, 0.97, 1.0), reflection);
    gl_FragColor = color;
}
";

#[derive(Clone, Copy, Debug)]
pub struct CursorWakeSegment {
    start: Point<f64, Logical>,
    end: Point<f64, Logical>,
    strength: f32,
}

#[derive(Debug, Default)]
pub struct CursorWakeTrail {
    last_sample: Option<(Point<f64, Logical>, Instant)>,
    last_disturbance: Option<Instant>,
    samples: VecDeque<CursorWakeSample>,
}

#[derive(Clone, Copy, Debug)]
struct CursorWakeSample {
    position: Point<f64, Logical>,
    created_at: Instant,
    strength: f32,
}

#[derive(Clone, Debug)]
pub struct CursorWake {
    segments: Vec<(CursorWakeSegment, f32)>,
}

#[derive(Debug, Default)]
pub struct CursorWakeFrame {
    screen: Option<GlesTexture>,
    draw_reported: bool,
}

#[derive(Clone, Debug)]
pub struct CursorWakePrograms {
    ribbon: RibbonProgram,
}

#[derive(Clone, Debug)]
struct RibbonProgram {
    id: u32,
    position: i32,
    normal: i32,
    side: i32,
    opacity: i32,
    screen_size: i32,
    screen_tex: i32,
    distortion_strength: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RibbonVertex {
    position: [f32; 2],
    normal: [f32; 2],
    side: f32,
    opacity: f32,
}

#[derive(Clone, Debug)]
pub struct CursorWakeElement {
    id: Id,
    commit: CommitCounter,
    geometry: Rectangle<i32, Logical>,
    programs: CursorWakePrograms,
    wake: CursorWake,
    wake_width: f32,
    cursor_size: f32,
    distortion_strength: f32,
    cache: std::rc::Rc<RefCell<CursorWakeFrame>>,
}

impl CursorWakeElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        commit: CommitCounter,
        geometry: Rectangle<i32, Logical>,
        programs: CursorWakePrograms,
        wake: CursorWake,
        wake_width: f32,
        cursor_size: f32,
        distortion_strength: f32,
        cache: std::rc::Rc<RefCell<CursorWakeFrame>>,
    ) -> Self {
        Self {
            id,
            commit,
            geometry,
            programs,
            wake,
            wake_width,
            cursor_size,
            distortion_strength,
            cache,
        }
    }

    pub fn capture(&self, frame: &mut GlesFrame<'_, '_>) -> Result<(), GlesError> {
        capture_cursor_wake_screen(frame, &mut self.cache.borrow_mut())
    }
}

impl Element for CursorWakeElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            self.geometry
                .size
                .to_f64()
                .to_buffer(1.0, Transform::Normal),
        )
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        _commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        smithay::backend::renderer::utils::DamageSet::from_slice(&[self.geometry(scale)])
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for CursorWakeElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        capture_cursor_wake_screen(frame, &mut self.cache.borrow_mut())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        draw_cursor_wake(
            frame,
            &self.programs,
            &self.wake,
            &mut self.cache.borrow_mut(),
            self.wake_width,
            self.cursor_size,
            self.distortion_strength,
        )
    }
}

impl CursorWakeTrail {
    pub fn record_motion(
        &mut self,
        position: Point<f64, Logical>,
        now: Instant,
        enabled: bool,
        threshold: f32,
    ) {
        if !enabled {
            self.clear();
            return;
        }
        let Some((previous, previous_at)) = self.last_sample else {
            self.last_sample = Some((position, now));
            return;
        };
        let delta = position - previous;
        let distance = delta.x.hypot(delta.y);
        if distance < WAKE_MIN_SAMPLE_DISTANCE {
            return;
        }
        self.last_sample = Some((position, now));
        let seconds = now.saturating_duration_since(previous_at).as_secs_f64();
        if seconds <= f64::EPSILON {
            return;
        }
        let speed = distance / seconds;
        let threshold = f64::from(threshold);
        #[allow(clippy::cast_possible_truncation)]
        let strength = ((speed / threshold - 1.0) / 1.5).clamp(0.0, 1.0) as f32;
        tracing::debug!(
            speed,
            threshold,
            strength,
            distance,
            "cursor wake motion sample"
        );
        if strength <= 0.0 {
            // Keep a short deceleration section connected to an already active
            // gesture. Threshold crossings inside one physical mouse sweep must
            // not split the ribbon into independent one-segment fragments.
            if self
                .last_disturbance
                .is_some_and(|last| now.saturating_duration_since(last) < WAKE_GESTURE_GAP)
                && !self.samples.is_empty()
            {
                let trailing_strength = self
                    .samples
                    .back()
                    .map_or(0.0, |sample| sample.strength * 0.75);
                self.samples.push_back(CursorWakeSample {
                    position,
                    created_at: now,
                    strength: trailing_strength,
                });
                while self.samples.len() > WAKE_MAX_SAMPLES {
                    self.samples.pop_front();
                }
                self.prune(now, WAKE_LIFETIME);
            }
            return;
        }
        let starts_new_gesture = self
            .last_disturbance
            .is_none_or(|last| now.saturating_duration_since(last) >= WAKE_GESTURE_GAP);
        if starts_new_gesture {
            tracing::info!(speed, threshold, strength, "cursor wake gesture detected");
        }
        self.last_disturbance = Some(now);
        if self.samples.is_empty() {
            self.samples.push_back(CursorWakeSample {
                position: previous,
                created_at: previous_at,
                strength,
            });
        }
        self.samples.push_back(CursorWakeSample {
            position,
            created_at: now,
            strength,
        });
        while self.samples.len() > WAKE_MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.prune(now, WAKE_LIFETIME);
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn active_wake(&mut self, now: Instant, settle_time: Duration) -> Option<CursorWake> {
        self.prune(now, settle_time);
        let active = self
            .last_disturbance
            .is_some_and(|last| now.saturating_duration_since(last) < settle_time);
        if !active {
            return None;
        }
        let segments = smoothed_wake_segments(&self.samples, now, settle_time);
        (!segments.is_empty()).then_some(CursorWake { segments })
    }

    pub fn clear(&mut self) {
        self.last_sample = None;
        self.last_disturbance = None;
        self.samples.clear();
    }

    fn prune(&mut self, now: Instant, lifetime: Duration) {
        while self
            .samples
            .front()
            .is_some_and(|sample| now.saturating_duration_since(sample.created_at) >= lifetime)
        {
            self.samples.pop_front();
        }
    }
}

#[allow(clippy::cast_possible_truncation)] // Normalized animation age is a shader f32.
fn smoothed_wake_segments(
    samples: &VecDeque<CursorWakeSample>,
    now: Instant,
    lifetime: Duration,
) -> Vec<(CursorWakeSegment, f32)> {
    if samples.len() < 2 {
        return Vec::new();
    }
    let selected = samples.iter().copied().collect::<Vec<_>>();
    let mut result = Vec::with_capacity(selected.len() - 1);
    for pair in selected.windows(2) {
        let end = pair[1];
        let age =
            now.saturating_duration_since(end.created_at).as_secs_f64() / lifetime.as_secs_f64();
        result.push((
            CursorWakeSegment {
                start: pair[0].position,
                end: end.position,
                strength: f32::midpoint(pair[0].strength, end.strength),
            },
            age.clamp(0.0, 1.0) as f32,
        ));
    }
    result
}

#[allow(clippy::borrow_as_ptr)] // GLES FFI requires raw out-pointers.
fn compile_ribbon_program(renderer: &mut GlesRenderer) -> Result<RibbonProgram, GlesError> {
    renderer.with_context(|gl| unsafe {
        fn compile(gl: &ffi::Gles2, kind: u32, source: &str) -> Result<u32, GlesError> {
            unsafe {
                let shader = gl.CreateShader(kind);
                if shader == 0 {
                    return Err(GlesError::CreateShaderObject);
                }
                let source = CString::new(source).map_err(|_| GlesError::ShaderCompileError)?;
                let pointer = source.as_ptr();
                gl.ShaderSource(shader, 1, &pointer, std::ptr::null());
                gl.CompileShader(shader);
                let mut status = 0;
                gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut status);
                if status == 0 {
                    gl.DeleteShader(shader);
                    return Err(GlesError::ShaderCompileError);
                }
                Ok(shader)
            }
        }

        let vertex = compile(gl, ffi::VERTEX_SHADER, RIBBON_VERTEX_SHADER)?;
        let fragment = compile(gl, ffi::FRAGMENT_SHADER, RIBBON_FRAGMENT_SHADER)?;
        let id = gl.CreateProgram();
        gl.AttachShader(id, vertex);
        gl.AttachShader(id, fragment);
        gl.LinkProgram(id);
        gl.DeleteShader(vertex);
        gl.DeleteShader(fragment);
        let mut status = 0;
        gl.GetProgramiv(id, ffi::LINK_STATUS, &mut status);
        if status == 0 {
            gl.DeleteProgram(id);
            return Err(GlesError::ProgramLinkError);
        }
        let location = |name: &str, attribute: bool| {
            let name = CString::new(name).expect("static ribbon shader name has no NUL");
            if attribute {
                gl.GetAttribLocation(id, name.as_ptr())
            } else {
                gl.GetUniformLocation(id, name.as_ptr())
            }
        };
        Ok(RibbonProgram {
            id,
            position: location("position", true),
            normal: location("normal", true),
            side: location("side", true),
            opacity: location("opacity", true),
            screen_size: location("screen_size", false),
            screen_tex: location("screen_tex", false),
            distortion_strength: location("distortion_strength", false),
        })
    })?
}

pub fn compile_cursor_wake_shader(
    renderer: &mut GlesRenderer,
) -> Result<CursorWakePrograms, GlesError> {
    let ribbon = compile_ribbon_program(renderer)?;
    Ok(CursorWakePrograms { ribbon })
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
pub fn draw_cursor_wake(
    frame: &mut GlesFrame<'_, '_>,
    programs: &CursorWakePrograms,
    wake: &CursorWake,
    cache: &mut CursorWakeFrame,
    wake_width: f32,
    cursor_size: f32,
    distortion_strength: f32,
) -> Result<(), GlesError> {
    let size = frame
        .output_size()
        .to_logical(1)
        .to_buffer(1, Transform::Normal);
    let screen = cache.screen.as_ref().ok_or(GlesError::BlitError)?;
    let mut centers = Vec::with_capacity(wake.segments.len() + 1);
    for (index, (segment, age)) in wake.segments.iter().enumerate() {
        let start: Point<f64, Physical> = (segment.start.x, segment.start.y).into();
        let end: Point<f64, Physical> = (segment.end.x, segment.end.y).into();
        if index == 0 {
            centers.push((start.x as f32, start.y as f32, *age, segment.strength));
        }
        centers.push((end.x as f32, end.y as f32, *age, segment.strength));
    }
    let vertices = build_ribbon_vertices(&centers, wake_width, cursor_size);
    if !cache.draw_reported && vertices.len() >= 26 {
        let (min_x, max_x, min_y, max_y) = vertices.iter().fold(
            (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
            |(min_x, max_x, min_y, max_y), vertex| {
                (
                    min_x.min(vertex.position[0]),
                    max_x.max(vertex.position[0]),
                    min_y.min(vertex.position[1]),
                    max_y.max(vertex.position[1]),
                )
            },
        );
        tracing::info!(
            segments = wake.segments.len(),
            vertices = vertices.len(),
            min_x,
            max_x,
            min_y,
            max_y,
            wake_width,
            distortion_strength,
            "cursor wake triangle strip submitted"
        );
        cache.draw_reported = true;
    }
    render_ribbon(
        frame,
        screen,
        &programs.ribbon,
        size,
        &vertices,
        distortion_strength,
    )?;
    Ok(())
}

fn capture_cursor_wake_screen(
    frame: &mut GlesFrame<'_, '_>,
    cache: &mut CursorWakeFrame,
) -> Result<(), GlesError> {
    let size = frame
        .output_size()
        .to_logical(1)
        .to_buffer(1, Transform::Normal);
    if cache
        .screen
        .as_ref()
        .is_none_or(|texture| texture.size() != size)
    {
        let mut renderer = frame.renderer();
        cache.screen = Some(renderer.as_mut().create_buffer(Fourcc::Abgr8888, size)?);
    }
    let screen = cache.screen.as_mut().ok_or(GlesError::BlitError)?;
    let mut target = frame.renderer().as_mut().bind(screen)?;
    let area = Rectangle::from_size(frame.output_size());
    frame.blit_to(&mut target, area, area, TextureFilter::Linear)
}

#[allow(clippy::cast_precision_loss)] // Small bounded mesh indices become shader coordinates.
fn build_ribbon_vertices(
    samples: &[(f32, f32, f32, f32)],
    wake_width: f32,
    cursor_size: f32,
) -> Vec<RibbonVertex> {
    const SUBDIVISIONS: usize = 6;
    if samples.len() < 2 {
        return Vec::new();
    }
    let mut curve = Vec::with_capacity((samples.len() - 1) * SUBDIVISIONS + 1);
    for index in 0..samples.len() - 1 {
        let p0 = samples[index.saturating_sub(1)];
        let p1 = samples[index];
        let p2 = samples[index + 1];
        let p3 = samples.get(index + 2).copied().unwrap_or(p2);
        for step in 0..SUBDIVISIONS {
            let t = step as f32 / SUBDIVISIONS as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let interpolate = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * ((2.0 * b)
                    + (-a + c) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            curve.push((
                interpolate(p0.0, p1.0, p2.0, p3.0),
                interpolate(p0.1, p1.1, p2.1, p3.1),
                p1.2 + (p2.2 - p1.2) * t,
                p1.3 + (p2.3 - p1.3) * t,
            ));
        }
    }
    curve.push(*samples.last().expect("samples has at least two entries"));

    let mut vertices = Vec::with_capacity(curve.len() * 2);
    for index in 0..curve.len() {
        let previous = curve[index.saturating_sub(1)];
        let next = curve[(index + 1).min(curve.len() - 1)];
        let dx = next.0 - previous.0;
        let dy = next.1 - previous.1;
        let length = dx.hypot(dy).max(0.001);
        let normal = [-dy / length, dx / length];
        let age = curve[index].2.clamp(0.0, 1.0);
        let head_half_width = cursor_size.max(1.0) * 0.5;
        let half_width = head_half_width + wake_width * age * 7.0;
        let tail_progress = index as f32 / (curve.len() - 1) as f32;
        let tail_taper_position = (tail_progress / 0.18).clamp(0.0, 1.0);
        let tail_taper =
            tail_taper_position * tail_taper_position * (3.0 - 2.0 * tail_taper_position);
        let opacity = curve[index].3 * cursor_wake_fade(age) * tail_taper;
        for side in [-1.0_f32, 1.0] {
            vertices.push(RibbonVertex {
                position: [
                    curve[index].0 + normal[0] * half_width * side,
                    curve[index].1 + normal[1] * half_width * side,
                ],
                normal,
                side,
                opacity,
            });
        }
    }
    vertices
}

fn cursor_wake_fade(age: f32) -> f32 {
    let progress = age.clamp(0.0, 1.0).powf(1.35);
    let smooth_progress = progress * progress * (3.0 - 2.0 * progress);
    1.0 - smooth_progress
}

#[allow(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]
fn render_ribbon(
    frame: &mut GlesFrame<'_, '_>,
    screen: &GlesTexture,
    program: &RibbonProgram,
    screen_size: smithay::utils::Size<i32, Buffer>,
    vertices: &[RibbonVertex],
    distortion_strength: f32,
) -> Result<(), GlesError> {
    if !vertices.is_empty() {
        frame.with_context(|gl| unsafe {
            let mut previous_program = 0;
            let mut previous_buffer = 0;
            let mut previous_active_texture = 0;
            let mut previous_texture = 0;
            gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut previous_program);
            gl.GetIntegerv(ffi::ARRAY_BUFFER_BINDING, &mut previous_buffer);
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut previous_active_texture);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous_texture);
            let blend = gl.IsEnabled(ffi::BLEND) == ffi::TRUE;
            let scissor = gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE;
            gl.Disable(ffi::BLEND);
            gl.Disable(ffi::SCISSOR_TEST);
            gl.UseProgram(program.id);
            gl.BindTexture(ffi::TEXTURE_2D, screen.tex_id());
            // `create_buffer` allocates only level zero. GLES defaults the minification
            // filter to a mipmap mode, which makes that texture incomplete and causes
            // texture2D() to return black. Smithay's normal texture path sets these on
            // every draw; this raw triangle-strip path must do the same explicitly.
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.Uniform1i(program.screen_tex, 0);
            gl.Uniform2f(
                program.screen_size,
                screen_size.w as f32,
                screen_size.h as f32,
            );
            gl.Uniform1f(program.distortion_strength, distortion_strength);
            let mut buffer = 0;
            gl.GenBuffers(1, &mut buffer);
            gl.BindBuffer(ffi::ARRAY_BUFFER, buffer);
            gl.BufferData(
                ffi::ARRAY_BUFFER,
                size_of_val(vertices) as isize,
                vertices.as_ptr().cast(),
                ffi::STREAM_DRAW,
            );
            let stride = size_of::<RibbonVertex>() as i32;
            let locations = [
                program.position,
                program.normal,
                program.side,
                program.opacity,
            ];
            let mut attribute_state = Vec::with_capacity(locations.len());
            for location in locations {
                let location = location as u32;
                let mut enabled = 0;
                let mut divisor = 0;
                gl.GetVertexAttribiv(location, ffi::VERTEX_ATTRIB_ARRAY_ENABLED, &mut enabled);
                gl.GetVertexAttribiv(location, ffi::VERTEX_ATTRIB_ARRAY_DIVISOR, &mut divisor);
                attribute_state.push((location, enabled != 0, divisor));
                gl.VertexAttribDivisor(location, 0);
                gl.EnableVertexAttribArray(location);
            }
            gl.VertexAttribPointer(
                program.position as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                std::ptr::null(),
            );
            gl.VertexAttribPointer(
                program.normal as u32,
                2,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                (2 * size_of::<f32>()) as *const _,
            );
            gl.VertexAttribPointer(
                program.side as u32,
                1,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                (4 * size_of::<f32>()) as *const _,
            );
            gl.VertexAttribPointer(
                program.opacity as u32,
                1,
                ffi::FLOAT,
                ffi::FALSE,
                stride,
                (5 * size_of::<f32>()) as *const _,
            );
            gl.DrawArrays(ffi::TRIANGLE_STRIP, 0, vertices.len() as i32);
            let draw_error = gl.GetError();
            for (location, enabled, divisor) in attribute_state {
                gl.VertexAttribDivisor(location, divisor as u32);
                if enabled {
                    gl.EnableVertexAttribArray(location);
                } else {
                    gl.DisableVertexAttribArray(location);
                }
            }
            gl.DeleteBuffers(1, &buffer);
            gl.BindBuffer(
                ffi::ARRAY_BUFFER,
                u32::try_from(previous_buffer).unwrap_or_default(),
            );
            gl.BindTexture(
                ffi::TEXTURE_2D,
                u32::try_from(previous_texture).unwrap_or_default(),
            );
            gl.ActiveTexture(u32::try_from(previous_active_texture).unwrap_or(ffi::TEXTURE0));
            gl.UseProgram(u32::try_from(previous_program).unwrap_or_default());
            if blend {
                gl.Enable(ffi::BLEND);
            }
            if scissor {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            (draw_error == ffi::NO_ERROR)
                .then_some(())
                .ok_or(GlesError::BlitError)
        })??;
    }
    Ok(())
}

const SHADOW_SHADER: &str = r"
precision highp float;
uniform vec2 size;
uniform float alpha;
uniform vec4 window_rect;
uniform float corner_radius;
uniform float softness;
uniform vec4 shadow_color;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

float rounded_box_distance(vec2 point, vec2 half_size, float radius) {
    vec2 distance = abs(point) - (half_size - vec2(radius));
    return length(max(distance, 0.0))
        + min(max(distance.x, distance.y), 0.0) - radius;
}

void main() {
    vec2 point = v_coords * size;
    vec2 half_size = window_rect.zw * 0.5;
    float radius = min(corner_radius, min(half_size.x, half_size.y));
    float distance = rounded_box_distance(
        point - (window_rect.xy + half_size),
        half_size,
        radius
    );
    float outside = max(distance, 0.0);
    float falloff = exp(-(outside * outside) / (2.0 * softness * softness));
    float outside_mask = smoothstep(-0.75, 0.75, distance);
    float opacity = shadow_color.a * falloff * outside_mask * alpha;
    vec3 color = shadow_color.rgb * opacity;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec3(0.0, 0.2, 0.0) * opacity * 0.2 + color * 0.8;
#endif
    gl_FragColor = vec4(color, opacity);
}
";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowOptions {
    pub radius: f32,
    pub offset: [i32; 2],
    pub color: [f32; 4],
    pub corner_radius: u32,
}

pub fn compile_shadow_shader(renderer: &mut GlesRenderer) -> Result<GlesPixelProgram, GlesError> {
    renderer.compile_custom_pixel_shader(
        SHADOW_SHADER,
        &[
            UniformName::new("window_rect", UniformType::_4f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("softness", UniformType::_1f),
            UniformName::new("shadow_color", UniformType::_4f),
        ],
    )
}

#[derive(Clone, Debug)]
pub struct ShadowElement {
    id: Id,
    commit: CommitCounter,
    geometry: Rectangle<i32, Logical>,
    window_rect: Rectangle<i32, Logical>,
    program: GlesPixelProgram,
    options: ShadowOptions,
}

impl ShadowElement {
    pub fn new(
        id: Id,
        window_geometry: Rectangle<i32, Logical>,
        program: GlesPixelProgram,
        options: ShadowOptions,
    ) -> Self {
        let (geometry, window_rect) = shadow_geometry(window_geometry, options);
        let commit = shadow_commit(options);
        Self {
            id,
            commit,
            geometry,
            window_rect,
            program,
            options,
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn shadow_geometry(
    window_geometry: Rectangle<i32, Logical>,
    options: ShadowOptions,
) -> (Rectangle<i32, Logical>, Rectangle<i32, Logical>) {
    let extent = (options.radius * 3.0).ceil() as i32;
    let offset: Point<i32, Logical> = (options.offset[0], options.offset[1]).into();
    let shadow_rect = Rectangle::new(window_geometry.loc + offset, window_geometry.size);
    let left = shadow_rect.loc.x.min(window_geometry.loc.x) - extent;
    let top = shadow_rect.loc.y.min(window_geometry.loc.y) - extent;
    let right = (shadow_rect.loc.x + shadow_rect.size.w)
        .max(window_geometry.loc.x + window_geometry.size.w)
        + extent;
    let bottom = (shadow_rect.loc.y + shadow_rect.size.h)
        .max(window_geometry.loc.y + window_geometry.size.h)
        + extent;
    let geometry = Rectangle::new((left, top).into(), (right - left, bottom - top).into());
    let window_rect = Rectangle::new(shadow_rect.loc - geometry.loc, shadow_rect.size);
    (geometry, window_rect)
}

fn shadow_commit(options: ShadowOptions) -> CommitCounter {
    let mut hash = options.radius.to_bits() as usize;
    for value in options.offset {
        hash = hash.rotate_left(7)
            ^ usize::try_from(i64::from(value) - i64::from(i32::MIN)).unwrap_or_default();
    }
    for value in options.color {
        hash = hash.rotate_left(7) ^ value.to_bits() as usize;
    }
    hash = hash.rotate_left(7) ^ options.corner_radius as usize;
    CommitCounter::from(hash)
}

impl Element for ShadowElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            self.geometry
                .size
                .to_f64()
                .to_buffer(1.0, Transform::Normal),
        )
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }
}

impl RenderElement<GlesRenderer> for ShadowElement {
    #[allow(clippy::cast_precision_loss)]
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let rect = self.window_rect;
        frame.render_pixel_shader_to(
            &self.program,
            src,
            dst,
            self.geometry.size.to_buffer(1, Transform::Normal),
            Some(damage),
            1.0,
            &[
                Uniform::new(
                    "window_rect",
                    (
                        rect.loc.x as f32,
                        rect.loc.y as f32,
                        rect.size.w as f32,
                        rect.size.h as f32,
                    ),
                ),
                Uniform::new("corner_radius", self.options.corner_radius as f32),
                Uniform::new("softness", self.options.radius),
                Uniform::new("shadow_color", self.options.color),
            ],
        )
    }
}

const TEXTURE_SHADER_HEADER: &str = r"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 half_pixel;
uniform float offset;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
";

const DOWNSAMPLE_SHADER_BODY: &str = r"
void main() {
    vec2 step = half_pixel * offset;
    vec4 color = texture2D(tex, v_coords) * 4.0;
    color += texture2D(tex, v_coords + vec2(-step.x, -step.y));
    color += texture2D(tex, v_coords + vec2( step.x, -step.y));
    color += texture2D(tex, v_coords + vec2(-step.x,  step.y));
    color += texture2D(tex, v_coords + vec2( step.x,  step.y));
    color *= 0.125;

#if defined(NO_ALPHA)
    color.a = 1.0;
#endif
    color *= alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
";

const UPSAMPLE_SHADER_BODY: &str = r"
void main() {
    vec2 step = half_pixel * offset;
    vec2 half_step = step * 0.5;
    vec4 color = texture2D(tex, v_coords + vec2(-step.x, 0.0));
    color += texture2D(tex, v_coords + vec2( step.x, 0.0));
    color += texture2D(tex, v_coords + vec2(0.0, -step.y));
    color += texture2D(tex, v_coords + vec2(0.0,  step.y));
    color += texture2D(tex, v_coords + vec2(-half_step.x, -half_step.y)) * 2.0;
    color += texture2D(tex, v_coords + vec2( half_step.x, -half_step.y)) * 2.0;
    color += texture2D(tex, v_coords + vec2(-half_step.x,  half_step.y)) * 2.0;
    color += texture2D(tex, v_coords + vec2( half_step.x,  half_step.y)) * 2.0;
    color *= 0.0833333333;

#if defined(NO_ALPHA)
    color.a = 1.0;
#endif
    color *= alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
";

const ROUNDING_SHADER: &str = r"
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
uniform vec4 clip_rect;
uniform float corner_radius;
uniform float transition_progress;
uniform int transition_effect;
uniform int transition_direction;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
float hash(vec2 point) {
    return fract(sin(dot(point, vec2(12.9898, 78.233))) * 43758.5453);
}
float water_noise(vec2 point) {
    vec2 cell = floor(point);
    vec2 fraction = fract(point);
    fraction = fraction * fraction * (3.0 - 2.0 * fraction);
    return mix(
        mix(hash(cell), hash(cell + vec2(1.0, 0.0)), fraction.x),
        mix(hash(cell + vec2(0.0, 1.0)), hash(cell + vec2(1.0, 1.0)), fraction.x),
        fraction.y
    );
}
void main() {
    vec2 local = (gl_FragCoord.xy - clip_rect.xy) / max(clip_rect.zw, vec2(1.0));
    vec2 centered = local - vec2(0.5);
    float transition = clamp(transition_progress, 0.0, 1.0);
    vec4 color;
    float transition_mask;
    if (transition_effect == 2) {
        float horizontal = smoothstep(0.0, 0.28, transition);
        float vertical = smoothstep(0.22, 0.92, transition);
        float horizontal_mask = 1.0 - smoothstep(
            horizontal * 0.5,
            horizontal * 0.5 + 0.012,
            abs(centered.x)
        );
        float vertical_mask = 1.0 - smoothstep(
            vertical * 0.5,
            vertical * 0.5 + 0.012,
            abs(centered.y)
        );
        transition_mask = horizontal_mask * vertical_mask;
        color = texture2D(tex, v_coords);
        float line_distance = abs(centered.y) * clip_rect.w;
        float line = (1.0 - smoothstep(0.8, 2.8, line_distance))
            * horizontal_mask
            * (1.0 - smoothstep(0.24, 0.48, transition));
        color *= transition_mask;
        float combined_alpha = max(color.a, line);
        color.rgb = min(color.rgb + vec3(line), vec3(combined_alpha));
        color.a = combined_alpha;
    } else {
        float noise = water_noise(local * vec2(5.0, 3.0));
        float animated = min(abs(float(transition_direction)), 1.0);
        float disturbance = sin(transition * 3.14159265) * animated;
        vec2 liquid_phase = vec2(disturbance * 0.31, -disturbance * 0.23);
        float noise_x = water_noise(local * vec2(3.7, 4.3) + vec2(2.1, 0.7) + liquid_phase);
        float noise_y = water_noise(local * vec2(4.1, 3.5) + vec2(0.4, 2.8) - liquid_phase.yx);
        vec2 flow = vec2(
            (noise_x - 0.5) * 0.0160 + sin(local.y * 7.0 + noise * 2.0) * 0.0024,
            (noise_y - 0.5) * 0.0130 + sin(local.x * 6.0 - noise * 2.0) * 0.0020
        ) * disturbance;
        vec2 sample_coords = clamp(v_coords + flow, vec2(0.0), vec2(1.0));
        float softness = 0.0042 * disturbance;
        color = texture2D(tex, sample_coords) * 0.54;
        color += texture2D(tex, clamp(sample_coords + vec2(softness, softness * 0.35), vec2(0.0), vec2(1.0))) * 0.23;
        color += texture2D(tex, clamp(sample_coords - vec2(softness, softness * 0.35), vec2(0.0), vec2(1.0))) * 0.23;
        color.rgb *= mix(1.0, mix(0.94, 1.06, noise), disturbance);

        // The whole image is one water surface. Fade every part at the same time;
        // low-frequency variation only softens the image, never sweeps across it.
        float fade = smoothstep(0.02, 0.98, transition);
        float dissolve_texture = mix(1.0, mix(0.70, 1.0, noise), disturbance);
        transition_mask = fade * dissolve_texture;
    }
#if defined(NO_ALPHA)
    color.a = 1.0;
#endif
    vec2 point = gl_FragCoord.xy - clip_rect.xy;
    vec2 half_size = clip_rect.zw * 0.5;
    float radius = min(corner_radius, min(half_size.x, half_size.y));
    vec2 distance = abs(point - half_size) - (half_size - vec2(radius));
    float edge = length(max(distance, 0.0)) + min(max(distance.x, distance.y), 0.0) - radius;
    color *= 1.0 - smoothstep(-0.75, 0.75, edge);
    if (transition_effect != 2)
        color *= transition_mask;
    color *= alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
";

pub fn compile_rounding_shader(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        ROUNDING_SHADER,
        &[
            UniformName::new("clip_rect", UniformType::_4f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("transition_progress", UniformType::_1f),
            UniformName::new("transition_effect", UniformType::_1i),
            UniformName::new("transition_direction", UniformType::_1i),
        ],
    )
}

#[derive(Debug)]
pub struct RoundedElement<E> {
    inner: E,
    program: GlesTexProgram,
    clip: Rectangle<i32, Physical>,
    radius: f32,
    transition_progress: f32,
    transition_effect: i32,
    transition_direction: i32,
}

impl<E> RoundedElement<E> {
    pub fn new(
        inner: E,
        program: GlesTexProgram,
        clip: Rectangle<i32, Physical>,
        radius: f32,
        transition_progress: f32,
        transition_effect: i32,
        transition_direction: i32,
    ) -> Self {
        Self {
            inner,
            program,
            clip,
            radius,
            transition_progress,
            transition_effect,
            transition_direction,
        }
    }
}

impl<E: Element> Element for RoundedElement<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        if self.transition_progress < 1.0 {
            CommitCounter::from(self.transition_progress.to_bits() as usize)
        } else {
            self.inner.current_commit()
        }
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        if self.transition_progress < 1.0 {
            smithay::backend::renderer::utils::DamageSet::from_slice(&[Rectangle::from_size(
                self.geometry(scale).size,
            )])
        } else {
            self.inner.damage_since(scale, commit)
        }
    }
    fn opaque_regions(
        &self,
        _scale: Scale<f64>,
    ) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        smithay::backend::renderer::utils::OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> smithay::backend::renderer::element::Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for RoundedElement<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(
            self.program.clone(),
            rounding_uniforms(
                frame,
                self.clip,
                self.radius,
                self.transition_progress,
                self.transition_effect,
                self.transition_direction,
            ),
        );
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        frame.clear_tex_program_override();
        result
    }
}

#[allow(clippy::cast_precision_loss)]
fn rounding_uniforms(
    frame: &GlesFrame<'_, '_>,
    clip: Rectangle<i32, Physical>,
    radius: f32,
    transition_progress: f32,
    transition_effect: i32,
    transition_direction: i32,
) -> Vec<Uniform<'static>> {
    let output = Rectangle::from_size(frame.output_size());
    let clip = frame.transformation().transform_rect_in(clip, &output.size);
    vec![
        Uniform::new(
            "clip_rect",
            (
                clip.loc.x as f32,
                clip.loc.y as f32,
                clip.size.w as f32,
                clip.size.h as f32,
            ),
        ),
        Uniform::new("corner_radius", radius),
        Uniform::new("transition_progress", transition_progress),
        Uniform::new("transition_effect", transition_effect),
        Uniform::new("transition_direction", transition_direction),
    ]
}

#[derive(Clone, Debug)]
pub struct BlurPrograms {
    downsample: GlesTexProgram,
    upsample: GlesTexProgram,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurOptions {
    pub passes: u8,
    pub offset: f32,
}

pub fn compile_blur_shaders(renderer: &mut GlesRenderer) -> Result<BlurPrograms, GlesError> {
    let uniforms = [
        UniformName::new("half_pixel", UniformType::_2f),
        UniformName::new("offset", UniformType::_1f),
    ];
    let downsample = renderer.compile_custom_texture_shader(
        format!("{TEXTURE_SHADER_HEADER}{DOWNSAMPLE_SHADER_BODY}"),
        &uniforms,
    )?;
    let upsample = renderer.compile_custom_texture_shader(
        format!("{TEXTURE_SHADER_HEADER}{UPSAMPLE_SHADER_BODY}"),
        &uniforms,
    )?;
    Ok(BlurPrograms {
        downsample,
        upsample,
    })
}

#[derive(Debug, Default)]
struct BlurCache {
    texture: Option<GlesTexture>,
    levels: Vec<GlesTexture>,
}

#[derive(Clone, Debug)]
pub struct BackdropBlurElement {
    id: Id,
    commit: CommitCounter,
    geometry: Rectangle<i32, Logical>,
    programs: BlurPrograms,
    options: BlurOptions,
    rounding: Option<(GlesTexProgram, f32, f32, i32, i32)>,
}

impl BackdropBlurElement {
    #[allow(clippy::cast_sign_loss)] // Bit patterns intentionally feed an opaque commit hash.
    pub fn new(
        id: Id,
        geometry: Rectangle<i32, Logical>,
        programs: BlurPrograms,
        options: BlurOptions,
        rounding: Option<(GlesTexProgram, f32, f32, i32, i32)>,
    ) -> Self {
        let (transition, direction) = rounding
            .as_ref()
            .map_or((1.0_f32, 0_i32), |(_, _, transition, _, direction)| {
                (*transition, *direction)
            });
        let commit = usize::from(options.passes)
            ^ (usize::try_from(options.offset.to_bits()).unwrap_or_default() << 8)
            ^ transition.to_bits() as usize
            ^ direction as usize;
        Self {
            id,
            commit: CommitCounter::from(commit),
            geometry,
            programs,
            options,
            rounding,
        }
    }
}

impl Element for BackdropBlurElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(
            self.geometry
                .size
                .to_f64()
                .to_buffer(1.0, Transform::Normal),
        )
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry.to_physical_precise_round(scale)
    }

    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

impl RenderElement<GlesRenderer> for BackdropBlurElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let output = Rectangle::from_size(frame.output_size());
        let Some(clamped) = dst.intersection(output) else {
            return Ok(());
        };
        let size = clamped.size.to_logical(1).to_buffer(1, Transform::Normal);
        let inner = cache.get_or_insert::<RefCell<BlurCache>, _>(RefCell::default);
        let mut inner = inner.borrow_mut();

        if inner
            .texture
            .as_ref()
            .is_none_or(|texture| texture.size() != size)
        {
            let mut renderer = frame.renderer();
            inner.texture = Some(renderer.as_mut().create_buffer(Fourcc::Abgr8888, size)?);
            inner.levels.clear();
        }
        let Some(texture) = inner.texture.as_ref() else {
            return Ok(());
        };
        let transformed = frame
            .transformation()
            .transform_rect_in(clamped, &output.size);

        frame.with_context(|gl| unsafe {
            while gl.GetError() != ffi::NO_ERROR {}
            let mut current_fbo = 0;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &raw mut current_fbo);
            gl.Disable(ffi::SCISSOR_TEST);
            let mut fbo = 0;
            gl.GenFramebuffers(1, &raw mut fbo);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                texture.tex_id(),
                0,
            );
            gl.BlitFramebuffer(
                transformed.loc.x,
                transformed.loc.y,
                transformed.loc.x + transformed.size.w,
                transformed.loc.y + transformed.size.h,
                0,
                0,
                size.w,
                size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );
            gl.BindFramebuffer(
                ffi::DRAW_FRAMEBUFFER,
                u32::try_from(current_fbo).unwrap_or_default(),
            );
            gl.Enable(ffi::SCISSOR_TEST);
            gl.DeleteFramebuffers(1, &raw const fbo);
            (gl.GetError() == ffi::NO_ERROR)
                .then_some(())
                .ok_or(GlesError::BlitError)
        })??;

        let mut renderer = frame.renderer();
        prepare_levels(renderer.as_mut(), &mut inner, size, self.options.passes)?;
        apply_dual_kawase(
            renderer.as_mut(),
            &mut inner,
            &self.programs,
            self.options.offset,
        )
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let Some(texture) = cache
            .and_then(UserDataMap::get::<RefCell<BlurCache>>)
            .and_then(|cache| cache.borrow().texture.clone())
        else {
            return Ok(());
        };
        let output = Rectangle::from_size(frame.output_size());
        let Some(clamped) = dst.intersection(output) else {
            return Ok(());
        };
        if let Some((
            program,
            radius,
            transition_progress,
            transition_effect,
            transition_direction,
        )) = &self.rounding
        {
            frame.override_default_tex_program(
                program.clone(),
                rounding_uniforms(
                    frame,
                    dst,
                    *radius,
                    *transition_progress,
                    *transition_effect,
                    *transition_direction,
                ),
            );
        }
        let result = frame.render_texture_from_to(
            &texture,
            Rectangle::from_size(texture.size().to_f64()),
            clamped,
            damage,
            &[],
            frame.transformation().invert(),
            1.0,
            None,
            &[],
        );
        if self.rounding.is_some() {
            frame.clear_tex_program_override();
        }
        result
    }
}

fn prepare_levels(
    renderer: &mut GlesRenderer,
    cache: &mut BlurCache,
    full_size: smithay::utils::Size<i32, Buffer>,
    passes: u8,
) -> Result<(), GlesError> {
    let mut expected = Vec::with_capacity(usize::from(passes));
    let mut size = full_size;
    for _ in 0..passes {
        size = ((size.w / 2).max(1), (size.h / 2).max(1)).into();
        expected.push(size);
    }
    if cache.levels.len() == expected.len()
        && cache
            .levels
            .iter()
            .zip(&expected)
            .all(|(texture, size)| texture.size() == *size)
    {
        return Ok(());
    }
    cache.levels.clear();
    for size in expected {
        cache
            .levels
            .push(renderer.create_buffer(Fourcc::Abgr8888, size)?);
    }
    Ok(())
}

fn apply_dual_kawase(
    renderer: &mut GlesRenderer,
    cache: &mut BlurCache,
    programs: &BlurPrograms,
    offset: f32,
) -> Result<(), GlesError> {
    let Some(full) = cache.texture.as_mut() else {
        return Ok(());
    };
    if cache.levels.is_empty() {
        return Ok(());
    }

    for index in 0..cache.levels.len() {
        let source = if index == 0 {
            full.clone()
        } else {
            cache.levels[index - 1].clone()
        };
        render_pass(
            renderer,
            &source,
            &mut cache.levels[index],
            &programs.downsample,
            offset,
        )?;
    }
    for index in (1..cache.levels.len()).rev() {
        let source = cache.levels[index].clone();
        render_pass(
            renderer,
            &source,
            &mut cache.levels[index - 1],
            &programs.upsample,
            offset,
        )?;
    }
    let source = cache.levels[0].clone();
    render_pass(renderer, &source, full, &programs.upsample, offset)
}

#[allow(clippy::cast_precision_loss)]
fn render_pass(
    renderer: &mut GlesRenderer,
    source: &GlesTexture,
    destination: &mut GlesTexture,
    program: &GlesTexProgram,
    offset: f32,
) -> Result<(), GlesError> {
    let source_size = source.size();
    let destination_size = destination.size();
    let physical_size = destination_size
        .to_logical(1, Transform::Normal)
        .to_physical(1);
    let damage = Rectangle::from_size(physical_size);
    let half_pixel = blur_half_pixel(source_size.w, source_size.h);
    let uniforms = [
        Uniform::new("half_pixel", half_pixel),
        Uniform::new("offset", offset),
    ];
    let mut framebuffer = renderer.bind(destination)?;
    let mut frame = renderer.render(&mut framebuffer, physical_size, Transform::Normal)?;
    frame.render_texture_from_to(
        source,
        Rectangle::from_size(source_size.to_f64()),
        damage,
        &[damage],
        &[],
        Transform::Normal,
        1.0,
        Some(program),
        &uniforms,
    )?;
    frame.finish().map(drop)
}

#[allow(clippy::cast_precision_loss)]
fn blur_half_pixel(source_width: i32, source_height: i32) -> (f32, f32) {
    (
        0.5 / source_width.max(1) as f32,
        0.5 / source_height.max(1) as f32,
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn blur_sampling_uses_the_source_texel_grid() {
        assert_eq!(blur_half_pixel(800, 600), (0.000_625, 1.0 / 1200.0));
        assert_eq!(blur_half_pixel(0, 0), (0.5, 0.5));
    }

    #[test]
    fn window_border_shader_exposes_appearance_controls() {
        for uniform in ["border_width", "corner_radius", "border_color"] {
            assert!(WINDOW_BORDER_SHADER.contains(uniform));
        }
    }

    #[test]
    fn focus_glow_tracks_the_presented_bottom_edge() {
        let window = Rectangle::new((300, 200).into(), (640, 480).into());
        assert_eq!(
            focus_glow_geometry(window, 3),
            Rectangle::new((300, 668).into(), (640, 48).into())
        );
    }

    #[test]
    fn focus_glow_shader_declares_every_custom_input() {
        for declaration in [
            "uniform vec2 size;",
            "uniform vec4 glow_color;",
            "uniform float line_height;",
            "uniform float glow_width;",
            "uniform float reveal;",
        ] {
            assert!(FOCUS_GLOW_SHADER.contains(declaration));
        }
    }

    #[test]
    fn water_shader_declares_its_configurable_uniforms() {
        assert!(RIBBON_VERTEX_SHADER.contains("attribute vec2 position;"));
        assert!(RIBBON_VERTEX_SHADER.contains("attribute vec2 normal;"));
        assert!(RIBBON_VERTEX_SHADER.contains("attribute float side;"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("vec2 tangent"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("value_noise(along)"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("edge_emphasis"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("outer_fade"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("distortion_profile"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("uniform float distortion_strength;"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("distortion_strength * 320.0"));
        assert!(RIBBON_FRAGMENT_SHADER.contains("texture2D(screen_tex"));
        assert!(ROUNDING_SHADER.contains("uniform float transition_progress;"));
        assert!(ROUNDING_SHADER.contains("uniform int transition_effect;"));
        assert!(ROUNDING_SHADER.contains("uniform int transition_direction;"));
        assert!(ROUNDING_SHADER.contains("float water_noise(vec2 point)"));
        assert!(ROUNDING_SHADER.contains("The whole image is one water surface"));
        assert!(ROUNDING_SHADER.contains("transition_mask = fade * dissolve_texture"));
        assert!(!ROUNDING_SHADER.contains("float waterline"));
        assert!(!ROUNDING_SHADER.contains("float surface_y"));
        assert!(!ROUNDING_SHADER.contains("float closing ="));
        assert!(!ROUNDING_SHADER.contains("normalize(centered"));
    }

    #[test]
    fn cursor_wake_mesh_is_one_continuous_triangle_strip() {
        let vertices = build_ribbon_vertices(
            &[
                (0.0, 0.0, 0.8, 0.5),
                (20.0, 0.0, 0.4, 0.8),
                (40.0, 20.0, 0.0, 1.0),
            ],
            12.0,
            24.0,
        );
        assert_eq!(vertices.len() % 2, 0);
        assert!(vertices.len() > 6);
        assert!(vertices.chunks_exact(2).all(|pair| {
            pair[0].side == -1.0 && pair[1].side == 1.0 && pair[0].normal == pair[1].normal
        }));
        let first_width = (vertices[0].position[1] - vertices[1].position[1]).abs();
        let last = vertices.len() - 2;
        let last_width = (vertices[last].position[0] - vertices[last + 1].position[0])
            .hypot(vertices[last].position[1] - vertices[last + 1].position[1]);
        assert!(first_width > last_width);
        assert!((last_width - 24.0).abs() < 0.01);
    }

    #[test]
    fn cursor_wake_fade_eases_gently_to_zero() {
        assert_eq!(cursor_wake_fade(0.0), 1.0);
        assert_eq!(cursor_wake_fade(1.0), 0.0);
        assert!(cursor_wake_fade(0.5) > 0.5);
        assert!(cursor_wake_fade(0.9) < 0.08);
        assert!(cursor_wake_fade(0.99) < 0.001);
    }

    #[test]
    fn shadow_bounds_include_negative_world_coordinates_and_offset() {
        let window = Rectangle::new((-100, -50).into(), (200, 100).into());
        let options = ShadowOptions {
            radius: 10.0,
            offset: [5, -7],
            color: [0.0, 0.0, 0.0, 0.5],
            corner_radius: 16,
        };

        let (geometry, local_window) = shadow_geometry(window, options);

        assert_eq!(
            geometry,
            Rectangle::new((-130, -87).into(), (265, 167).into())
        );
        assert_eq!(
            local_window,
            Rectangle::new((35, 30).into(), (200, 100).into())
        );
    }

    #[test]
    fn zero_radius_without_offset_does_not_expand_bounds() {
        let window = Rectangle::new((12, 34).into(), (200, 100).into());
        let options = ShadowOptions {
            radius: 0.0,
            offset: [0, 0],
            color: [0.0, 0.0, 0.0, 0.5],
            corner_radius: 16,
        };

        let (geometry, local_window) = shadow_geometry(window, options);

        assert_eq!(geometry, window);
        assert_eq!(local_window, Rectangle::from_size(window.size));
    }

    #[test]
    fn cursor_wake_ignores_slow_motion_and_scales_fast_motion() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 900.0);
        trail.record_motion(
            (10.0, 0.0).into(),
            start + Duration::from_millis(100),
            true,
            900.0,
        );
        assert!(trail
            .active_wake(
                start + Duration::from_millis(100),
                Duration::from_millis(1400)
            )
            .is_none());

        trail.record_motion(
            (30.0, 0.0).into(),
            start + Duration::from_millis(105),
            true,
            900.0,
        );
        let wake = trail
            .active_wake(
                start + Duration::from_millis(105),
                Duration::from_millis(1400),
            )
            .unwrap();
        assert_eq!(wake.segments.len(), 1);
        assert!(wake.segments[0].0.strength > 0.0);
        assert!(wake.segments[0].0.strength <= 1.0);
    }

    #[test]
    fn cursor_wake_preserves_recorded_direction_changes() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 100.0);
        trail.record_motion(
            (20.0, 0.0).into(),
            start + Duration::from_millis(10),
            true,
            100.0,
        );
        trail.record_motion(
            (20.0, 20.0).into(),
            start + Duration::from_millis(20),
            true,
            100.0,
        );

        let wake = trail
            .active_wake(
                start + Duration::from_millis(20),
                Duration::from_millis(1400),
            )
            .unwrap();
        assert_eq!(wake.segments.len(), 2);
        assert!(wake
            .segments
            .windows(2)
            .all(|pair| pair[0].0.end == pair[1].0.start));
        assert_eq!(wake.segments[0].0.start, (0.0, 0.0).into());
        assert_eq!(wake.segments[0].0.end, (20.0, 0.0).into());
        assert_eq!(wake.segments[1].0.start, (20.0, 0.0).into());
        assert_eq!(wake.segments[1].0.end, (20.0, 20.0).into());
        assert!(wake.segments[0].1 > wake.segments[1].1);
    }

    #[test]
    fn cursor_wake_does_not_truncate_a_normal_lifetime_at_48_samples() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 100.0);
        for index in 1_u32..=80 {
            trail.record_motion(
                (f64::from(index) * 10.0, 0.0).into(),
                start + Duration::from_millis(u64::from(index) * 10),
                true,
                100.0,
            );
        }

        let wake = trail
            .active_wake(
                start + Duration::from_millis(800),
                Duration::from_millis(1400),
            )
            .unwrap();
        assert_eq!(wake.segments.len(), 80);
        assert_eq!(wake.segments[0].0.start, (0.0, 0.0).into());
    }

    #[test]
    fn cursor_wake_remains_visible_after_motion_slows() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 100.0);
        trail.record_motion(
            (20.0, 0.0).into(),
            start + Duration::from_millis(10),
            true,
            100.0,
        );
        trail.record_motion(
            (27.0, 0.0).into(),
            start + Duration::from_millis(110),
            true,
            100.0,
        );

        let wake = trail.active_wake(
            start + Duration::from_millis(110),
            Duration::from_millis(1400),
        );
        assert!(wake.is_some());
    }

    #[test]
    fn cursor_wake_keeps_recent_history_when_motion_restarts() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 100.0);
        trail.record_motion(
            (20.0, 0.0).into(),
            start + Duration::from_millis(10),
            true,
            100.0,
        );
        trail.record_motion(
            (400.0, 0.0).into(),
            start + Duration::from_millis(310),
            true,
            100.0,
        );

        let wake = trail
            .active_wake(
                start + Duration::from_millis(310),
                Duration::from_millis(1400),
            )
            .unwrap();
        assert_eq!(wake.segments.len(), 2);
    }

    #[test]
    fn cursor_wake_expires_and_can_be_disabled() {
        let start = Instant::now();
        let mut trail = CursorWakeTrail::default();
        trail.record_motion((0.0, 0.0).into(), start, true, 100.0);
        trail.record_motion(
            (10.0, 0.0).into(),
            start + Duration::from_millis(10),
            true,
            100.0,
        );
        assert!(trail
            .active_wake(
                start + Duration::from_millis(10),
                Duration::from_millis(1400)
            )
            .is_some());
        assert!(trail
            .active_wake(
                start + Duration::from_millis(10) + WAKE_LIFETIME,
                Duration::from_millis(1400),
            )
            .is_none());
        assert!(trail
            .active_wake(
                start + Duration::from_millis(1410),
                Duration::from_millis(1400),
            )
            .is_none());

        trail.record_motion((0.0, 0.0).into(), start, false, 100.0);
        assert!(trail.samples.is_empty());
        assert!(trail.last_sample.is_none());
        assert!(trail.last_disturbance.is_none());
    }
}
