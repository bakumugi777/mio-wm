use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use kdl::{KdlDocument, KdlNode, KdlValue};
use mio_core::{Direction, GridSize};
use xkbcommon::xkb;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub output: OutputConfig,
    pub appearance: Appearance,
    pub effects: Effects,
    pub animation_speed: f64,
    pub viewport: GridSize,
    pub initial_window_size: GridSize,
    pub startup_commands: Vec<Vec<String>>,
    pub edge_commands: Vec<EdgeCommand>,
    pub mouse: MouseConfig,
    pub bindings: Vec<KeyBinding>,
    pub window_rules: Vec<WindowRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output: OutputConfig::default(),
            appearance: Appearance::default(),
            effects: Effects::default(),
            animation_speed: 1.0,
            viewport: GridSize::new(8, 8).expect("default viewport is valid"),
            initial_window_size: GridSize::new(8, 8).expect("default window size is valid"),
            startup_commands: Vec::new(),
            edge_commands: vec![EdgeCommand {
                edge: Direction::Down,
                argv: vec!["wofi".into(), "--show".into(), "drun".into()],
            }],
            mouse: MouseConfig::default(),
            bindings: default_bindings(),
            window_rules: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputConfig {
    pub scale: f64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseConfig {
    /// Milliseconds of pointer inactivity before hiding it. Zero disables it.
    pub cursor_hide_delay_ms: u32,
    pub camera_pan: MouseButton,
    pub camera_zoom: MouseButton,
    /// Ordered chord: hold the first button, then press the second.
    pub move_window: [MouseButton; 2],
    pub resize_window: MouseButton,
    pub reset_window: MouseButton,
    pub reset_window_clicks: u8,
    /// Ordered chord: hold the first button, then press the second.
    pub toggle_floating: [MouseButton; 2],
    pub place_next: MouseButton,
    /// Hold the first button, then click the second `center_window_clicks` times.
    pub center_window: [MouseButton; 2],
    pub center_window_clicks: u8,
    /// Hold the first button, then click the second `close_window_clicks` times.
    pub close_window: [MouseButton; 2],
    pub close_window_clicks: u8,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            cursor_hide_delay_ms: 0,
            camera_pan: MouseButton::Right,
            camera_zoom: MouseButton::Right,
            move_window: [MouseButton::Right, MouseButton::Left],
            resize_window: MouseButton::Left,
            reset_window: MouseButton::Right,
            reset_window_clicks: 2,
            toggle_floating: [MouseButton::Right, MouseButton::Middle],
            place_next: MouseButton::Middle,
            center_window: [MouseButton::Right, MouseButton::Left],
            center_window_clicks: 1,
            close_window: [MouseButton::Right, MouseButton::Left],
            close_window_clicks: 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeCommand {
    pub edge: Direction,
    pub argv: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effects {
    pub blur_passes: u8,
    pub blur_offset: f32,
    pub shadow_radius: f32,
    pub shadow_offset: [i32; 2],
    pub shadow_color: [f32; 4],
    pub cursor_wake: bool,
    pub cursor_wake_threshold: f32,
    pub cursor_wake_strength: f32,
    pub cursor_wake_width: f32,
    pub cursor_wake_duration: u32,
    pub window_transition: WindowTransitionEffect,
    pub window_transition_duration: u32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            blur_passes: 3,
            blur_offset: 2.0,
            shadow_radius: 16.0,
            shadow_offset: [0, 0],
            shadow_color: [0.0, 0.0, 0.0, 0.25],
            cursor_wake: true,
            cursor_wake_threshold: 1600.0,
            cursor_wake_strength: 0.032,
            cursor_wake_width: 8.5,
            cursor_wake_duration: 1400,
            window_transition: WindowTransitionEffect::Water,
            window_transition_duration: 420,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowTransitionEffect {
    None,
    Water,
    SciFi,
}

impl WindowTransitionEffect {
    pub const fn shader_value(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Water => 1,
            Self::SciFi => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub background_color: [f32; 4],
    pub window_border_width: u32,
    pub window_border_color: [f32; 4],
    pub focus_indicator_width: u32,
    pub focus_indicator_height: u32,
    pub focus_indicator_color: [f32; 4],
    pub corner_radius: u32,
    pub gaps: u32,
    pub opacity: f32,
    pub opacity_toggle: [f32; 2],
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            background_color: [1.0, 1.0, 1.0, 1.0],
            window_border_width: 1,
            window_border_color: [1.0, 1.0, 1.0, 46.0 / 255.0],
            focus_indicator_width: 320,
            focus_indicator_height: 2,
            focus_indicator_color: [1.0, 1.0, 1.0, 1.0],
            corner_radius: 0,
            gaps: 24,
            opacity: 1.0,
            opacity_toggle: [1.0, 0.8],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyBinding {
    pub chord: KeyChord,
    pub action: ConfigAction,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct KeyChord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    pub key: Key,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Letter(char),
    Symbol(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigAction {
    Close,
    Camera(Direction),
    CameraNudge(Direction),
    CameraZoom(f64),
    CycleOutput,
    Focus(Direction),
    Move(Direction),
    Resize(Direction),
    PlaceNext(Direction),
    ToggleFloating,
    ToggleFullscreen,
    ToggleMaximized,
    ToggleWindowSize,
    ToggleOpacity,
    ToggleBlur,
    ToggleCursorWake,
    ClearOpacity,
    ToggleOverview,
    SelectOverview,
    ReloadConfig,
    Spawn(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowRule {
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub opacity: Option<f32>,
    pub floating: Option<bool>,
    pub blur: Option<bool>,
}

#[derive(Debug)]
pub struct ConfigManager {
    config: Config,
    path: Option<PathBuf>,
}

impl ConfigManager {
    pub fn load(path: Option<PathBuf>, explicit: bool) -> Result<Self, ConfigError> {
        let path = path.or_else(default_path);
        let config = match path.as_deref() {
            Some(path) if path.exists() => parse_file(path)?,
            Some(path) if explicit => {
                return Err(ConfigError::new(format!(
                    "configuration file does not exist: {}",
                    path.display()
                )))
            }
            _ => Config::default(),
        };
        Ok(Self { config, path })
    }

    pub fn load_or_default(path: Option<PathBuf>, explicit: bool) -> (Self, Option<ConfigError>) {
        let retained_path = path.clone().or_else(default_path);
        match Self::load(path, explicit) {
            Ok(manager) => (manager, None),
            Err(error) => (
                Self {
                    config: Config::default(),
                    path: retained_path,
                },
                Some(error),
            ),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn reload(&mut self) -> Result<(), ConfigError> {
        let Some(path) = self.path.as_deref() else {
            self.config = Config::default();
            return Ok(());
        };
        if !path.exists() {
            return Err(ConfigError::new(format!(
                "configuration file does not exist: {}",
                path.display()
            )));
        }
        let replacement = parse_file(path)?;
        self.config = replacement;
        Ok(())
    }

    pub fn display_path(&self) -> String {
        self.path.as_deref().map_or_else(
            || "built-in defaults".into(),
            |path| path.display().to_string(),
        )
    }
}

fn default_path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join("mio/config.kdl"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/mio/config.kdl"))
}

fn parse_file(path: &Path) -> Result<Config, ConfigError> {
    let mut stack = Vec::new();
    let nodes = load_config_nodes(path, &mut stack)?;
    parse_nodes(nodes.iter().map(|loaded| {
        (
            &loaded.node,
            Some((
                loaded.path.as_path(),
                loaded.line,
                loaded.text.as_str(),
                loaded.source.as_ref(),
            )),
        )
    }))
}

#[cfg(test)]
fn parse(source: &str) -> Result<Config, ConfigError> {
    let document = parse_document(source, None)?;
    parse_nodes(document.nodes().iter().map(|node| (node, None)))
}

#[derive(Debug)]
struct LoadedNode {
    node: KdlNode,
    path: PathBuf,
    line: usize,
    text: String,
    source: Arc<str>,
}

fn parse_document(source: &str, path: Option<&Path>) -> Result<KdlDocument, ConfigError> {
    KdlDocument::from_str(source).map_err(|error| {
        let (line, column, text) = source_location(source, error.span.offset());
        let detail = format!(
            "KDL parse error at line {line}, column {column}: {}; `{}`",
            error.kind,
            text.trim()
        );
        ConfigError::new(path.map_or(detail.clone(), |path| {
            format!("{}: {detail}", path.display())
        }))
    })
}

fn load_config_nodes(
    path: &Path,
    stack: &mut Vec<PathBuf>,
) -> Result<Vec<LoadedNode>, ConfigError> {
    let logical = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                ConfigError::new(format!("failed to resolve current directory: {error}"))
            })?
            .join(path)
    };
    let canonical = fs::canonicalize(path)
        .map_err(|error| ConfigError::new(format!("failed to read {}: {error}", path.display())))?;
    if let Some(index) = stack.iter().position(|entry| entry == &canonical) {
        let mut cycle = stack[index..]
            .iter()
            .map(|entry| entry.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(canonical.display().to_string());
        return Err(ConfigError::new(format!(
            "configuration include cycle: {}",
            cycle.join(" -> ")
        )));
    }
    let source = fs::read_to_string(&canonical).map_err(|error| {
        ConfigError::new(format!("failed to read {}: {error}", canonical.display()))
    })?;
    let document = parse_document(&source, Some(&logical))?;
    let source: Arc<str> = source.into();
    stack.push(canonical.clone());
    let mut loaded = Vec::new();
    for node in document.nodes() {
        let (line, _, text) = source_location(&source, node.span().offset());
        if node.name().value() == "include" {
            if node.entries().len() != 1 || node.children().is_some() {
                return Err(ConfigError::new(format!(
                    "{}: include expects exactly one path at line {line}: `{}`",
                    logical.display(),
                    text.trim()
                )));
            }
            let include = node_string_at(node, 0).map_err(|error| {
                ConfigError::new(format!(
                    "{}: {error} at line {line}: `{}`",
                    logical.display(),
                    text.trim()
                ))
            })?;
            if include.is_empty() {
                return Err(ConfigError::new(format!(
                    "{}: include path cannot be empty at line {line}",
                    logical.display()
                )));
            }
            let include = PathBuf::from(include);
            let include = if include.is_absolute() {
                include
            } else {
                logical
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(include)
            };
            match fs::metadata(&include) {
                Ok(_) => loaded.extend(load_config_nodes(&include, stack)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ConfigError::new(format!(
                        "failed to inspect included configuration {}: {error}",
                        include.display()
                    )))
                }
            }
        } else {
            loaded.push(LoadedNode {
                node: node.clone(),
                path: logical.clone(),
                line,
                text: text.trim().to_owned(),
                source: Arc::clone(&source),
            });
        }
    }
    stack.pop();
    Ok(loaded)
}

fn parse_nodes<'a>(
    nodes: impl IntoIterator<Item = (&'a KdlNode, Option<(&'a Path, usize, &'a str, &'a str)>)>,
) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    let mut bindings = Vec::new();
    let mut saw_bind = false;
    let mut edge_commands = Vec::new();
    let mut saw_edge_command = false;
    let mut sources = Vec::new();

    for (node, source) in nodes {
        if let Some((path, _, _, full_source)) = source {
            sources.push((path, full_source));
        }
        let result = (|| -> Result<(), ConfigError> {
            match node.name().value() {
                "output" => parse_output(node, &mut config.output)?,
                "appearance" => parse_appearance(node, &mut config.appearance)?,
                "effects" => parse_effects(node, &mut config.effects)?,
                "animation" => config.animation_speed = child_number(node, "speed")?,
                "camera" => config.viewport = child_size(node, "viewport")?,
                "placement" => config.initial_window_size = child_size(node, "initial-size")?,
                "mouse" => parse_mouse(node, &mut config.mouse)?,
                "spawn-at-startup" => config.startup_commands.push(parse_command(node)?),
                "bind" => {
                    saw_bind = true;
                    bindings.push(parse_binding(node)?);
                }
                "edge-command" => {
                    saw_edge_command = true;
                    edge_commands.push(parse_edge_command(node)?);
                }
                "window-rule" => config.window_rules.push(parse_window_rule(node)?),
                "include" => {
                    return Err(ConfigError::new(
                        "include is only available when loading configuration from a file",
                    ))
                }
                name => return Err(ConfigError::new(format!("unknown top-level node `{name}`"))),
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(match source {
                Some((path, node_line, node_text, source)) => {
                    let (line, text) = error_line_hint(source, &error.to_string())
                        .unwrap_or((node_line, node_text));
                    ConfigError::new(format!(
                        "{}: {error} at line {line}: `{}`",
                        path.display(),
                        text.trim()
                    ))
                }
                None => error,
            });
        }
    }
    if saw_bind {
        let mut chords = HashSet::new();
        for binding in &bindings {
            if !chords.insert(binding.chord) {
                return Err(ConfigError::new(format!(
                    "duplicate key binding: {:?}",
                    binding.chord
                )));
            }
        }
        config.bindings = bindings;
    }
    if saw_edge_command {
        config.edge_commands = edge_commands;
    }
    let mut edges = HashSet::new();
    for command in &config.edge_commands {
        if !edges.insert(command.edge) {
            return Err(ConfigError::new(format!(
                "duplicate edge command: {:?}",
                command.edge
            )));
        }
    }
    if let Err(error) = validate(&config) {
        if let Some((path, line, text)) = sources.iter().rev().find_map(|(path, source)| {
            error_line_hint(source, &error.to_string()).map(|(line, text)| (*path, line, text))
        }) {
            return Err(ConfigError::new(format!(
                "{}: {error} at line {line}: `{}`",
                path.display(),
                text.trim()
            )));
        }
        return Err(error);
    }
    Ok(config)
}

fn parse_mouse(node: &KdlNode, mouse: &mut MouseConfig) -> Result<(), ConfigError> {
    let children = required_children(node)?;
    for child in children.nodes() {
        match child.name().value() {
            "cursor-hide-delay-ms" => {
                mouse.cursor_hide_delay_ms = u32::try_from(node_u64_at(child, 0)?)
                    .map_err(|_| ConfigError::new("mouse cursor-hide-delay-ms is too large"))?;
            }
            "camera-pan" => mouse.camera_pan = node_mouse_button_at(child, 0)?,
            "camera-zoom" => mouse.camera_zoom = node_mouse_button_at(child, 0)?,
            "move-window" => {
                mouse.move_window = [
                    node_mouse_button_at(child, 0)?,
                    node_mouse_button_at(child, 1)?,
                ];
            }
            "resize-window" => mouse.resize_window = node_mouse_button_at(child, 0)?,
            "reset-window" => {
                mouse.reset_window = node_mouse_button_at(child, 0)?;
                mouse.reset_window_clicks = u8::try_from(node_u64_property(child, "clicks")?)
                    .map_err(|_| ConfigError::new("mouse reset-window clicks is too large"))?;
            }
            "toggle-floating" => {
                mouse.toggle_floating = [
                    node_mouse_button_at(child, 0)?,
                    node_mouse_button_at(child, 1)?,
                ];
            }
            "place-next" => mouse.place_next = node_mouse_button_at(child, 0)?,
            "center-window" => {
                mouse.center_window = [
                    node_mouse_button_at(child, 0)?,
                    node_mouse_button_at(child, 1)?,
                ];
                mouse.center_window_clicks = u8::try_from(node_u64_property(child, "clicks")?)
                    .map_err(|_| ConfigError::new("mouse center-window clicks is too large"))?;
            }
            "close-window" => {
                mouse.close_window = [
                    node_mouse_button_at(child, 0)?,
                    node_mouse_button_at(child, 1)?,
                ];
                mouse.close_window_clicks = u8::try_from(node_u64_property(child, "clicks")?)
                    .map_err(|_| ConfigError::new("mouse close-window clicks is too large"))?;
            }
            name => return Err(ConfigError::new(format!("unknown mouse option `{name}`"))),
        }
    }
    validate_mouse(mouse)
}

fn parse_output(node: &KdlNode, output: &mut OutputConfig) -> Result<(), ConfigError> {
    let children = required_children(node)?;
    let mut saw_scale = false;
    for child in children.nodes() {
        match child.name().value() {
            "scale" if !saw_scale => {
                output.scale = node_number_at(child, 0)?;
                saw_scale = true;
            }
            "scale" => return Err(ConfigError::new("duplicate output option `scale`")),
            name => return Err(ConfigError::new(format!("unknown output option `{name}`"))),
        }
    }
    if !saw_scale {
        return Err(ConfigError::new("`output` requires `scale`"));
    }
    Ok(())
}

fn node_mouse_button_at(node: &KdlNode, index: usize) -> Result<MouseButton, ConfigError> {
    match node_string_at(node, index)?.to_ascii_lowercase().as_str() {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        value => Err(ConfigError::new(format!(
            "mouse button must be left, right, or middle, got `{value}`"
        ))),
    }
}

fn validate_mouse(mouse: &MouseConfig) -> Result<(), ConfigError> {
    if mouse.move_window[0] == mouse.move_window[1] {
        return Err(ConfigError::new(
            "mouse move-window requires two different buttons",
        ));
    }
    if mouse.reset_window != mouse.camera_pan {
        return Err(ConfigError::new(
            "mouse reset-window must use the camera-pan button so a possible multi-click can be deferred without leaking a client click",
        ));
    }
    if !(1..=5).contains(&mouse.reset_window_clicks) {
        return Err(ConfigError::new(
            "mouse reset-window clicks must be between 1 and 5",
        ));
    }
    if mouse.toggle_floating[0] == mouse.toggle_floating[1] {
        return Err(ConfigError::new(
            "mouse toggle-floating requires two different buttons",
        ));
    }
    let close = mouse.close_window;
    if close[0] == close[1] {
        return Err(ConfigError::new(
            "mouse close-window requires two different buttons; hold the first and click the second the configured number of times",
        ));
    }
    if close[0] != mouse.camera_pan
        || close != mouse.move_window
        || mouse.center_window != mouse.move_window
    {
        return Err(ConfigError::new(
            "mouse center-window and close-window must match move-window and start with camera-pan so pointer motion can safely disambiguate their Actions",
        ));
    }
    if !(1..=5).contains(&mouse.center_window_clicks) {
        return Err(ConfigError::new(
            "mouse center-window clicks must be between 1 and 5",
        ));
    }
    if !(1..=5).contains(&mouse.close_window_clicks) {
        return Err(ConfigError::new(
            "mouse close-window clicks must be between 1 and 5",
        ));
    }
    if mouse.center_window_clicks == mouse.close_window_clicks {
        return Err(ConfigError::new(
            "mouse center-window and close-window cannot use the same button sequence and click count",
        ));
    }
    Ok(())
}

fn parse_edge_command(node: &KdlNode) -> Result<EdgeCommand, ConfigError> {
    let edge = match node_string_at(node, 0)? {
        "left" => Direction::Left,
        "right" => Direction::Right,
        "top" => Direction::Up,
        "bottom" => Direction::Down,
        _ => {
            return Err(ConfigError::new(
                "edge-command edge must be left, right, top, or bottom",
            ))
        }
    };
    let argv = (1..node.entries().len())
        .map(|index| node_string_at(node, index).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    if argv.is_empty() || argv[0].is_empty() {
        return Err(ConfigError::new(
            "edge-command expects an executable after the edge",
        ));
    }
    Ok(EdgeCommand { edge, argv })
}

fn parse_command(node: &KdlNode) -> Result<Vec<String>, ConfigError> {
    parse_argv(node, 0, "spawn-at-startup")
}

fn parse_argv(node: &KdlNode, start: usize, context: &str) -> Result<Vec<String>, ConfigError> {
    let argv = (start..node.entries().len())
        .map(|index| node_string_at(node, index).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    if argv.first().is_none_or(String::is_empty) {
        return Err(ConfigError::new(format!(
            "{context} expects a non-empty executable"
        )));
    }
    Ok(argv)
}

fn source_location(source: &str, byte_offset: usize) -> (usize, usize, &str) {
    let offset = byte_offset.min(source.len());
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let column = source[line_start..offset].chars().count() + 1;
    let line_end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    (line, column, &source[line_start..line_end])
}

fn error_line_hint<'a>(source: &'a str, error: &str) -> Option<(usize, &'a str)> {
    let quoted = error
        .split_once('`')
        .and_then(|(_, rest)| rest.split_once('`'))
        .map(|(name, _)| name);
    let keyword = quoted.or_else(|| {
        [
            "spawn-at-startup",
            "background-color",
            "window-border-width",
            "window-border-color",
            "focus-indicator-color",
            "focus-indicator-width",
            "focus-indicator-height",
            "corner-radius",
            "gaps",
            "opacity-toggle",
            "opacity",
            "speed",
            "floating",
            "blur",
            "blur-passes",
            "blur-offset",
            "shadow-radius",
            "shadow-offset",
            "shadow-color",
            "cursor-wake",
            "cursor-wake-threshold",
            "cursor-wake-strength",
            "cursor-wake-width",
            "cursor-wake-duration",
            "window-transition",
            "window-transition-duration",
            "camera-pan",
            "camera-zoom",
            "move-window",
            "resize-window",
            "reset-window",
            "center-window",
            "toggle-floating",
            "place-next",
            "close-window",
        ]
        .into_iter()
        .find(|keyword| error.contains(keyword))
    })?;
    source
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(keyword))
        .map(|(index, line)| (index + 1, line))
}

fn parse_effects(node: &KdlNode, effects: &mut Effects) -> Result<(), ConfigError> {
    let children = required_children(node)?;
    for child in children.nodes() {
        match child.name().value() {
            "blur-passes" => {
                effects.blur_passes = u8::try_from(node_u32(child)?)
                    .map_err(|_| ConfigError::new("blur-passes is too large"))?;
            }
            "blur-offset" => {
                #[allow(clippy::cast_possible_truncation)]
                let value = node_number(child)? as f32;
                effects.blur_offset = value;
            }
            "shadow-radius" => {
                #[allow(clippy::cast_possible_truncation)]
                let value = node_number(child)? as f32;
                effects.shadow_radius = value;
            }
            "shadow-offset" => {
                effects.shadow_offset = [node_i32_at(child, 0)?, node_i32_at(child, 1)?];
            }
            "shadow-color" => effects.shadow_color = parse_color(node_string(child)?)?,
            "cursor-wake" => effects.cursor_wake = node_bool(child)?,
            "cursor-wake-threshold" => {
                #[allow(clippy::cast_possible_truncation)]
                let value = node_number(child)? as f32;
                effects.cursor_wake_threshold = value;
            }
            "cursor-wake-strength" => {
                #[allow(clippy::cast_possible_truncation)]
                let value = node_number(child)? as f32;
                effects.cursor_wake_strength = value;
            }
            "cursor-wake-width" => {
                #[allow(clippy::cast_possible_truncation)]
                let value = node_number(child)? as f32;
                effects.cursor_wake_width = value;
            }
            "cursor-wake-duration" => effects.cursor_wake_duration = node_u32(child)?,
            "window-transition" => effects.window_transition = node_window_transition(child)?,
            "window-transition-duration" => {
                effects.window_transition_duration = node_u32(child)?;
            }
            name => return Err(ConfigError::new(format!("unknown effects option `{name}`"))),
        }
    }
    Ok(())
}

fn parse_appearance(node: &KdlNode, appearance: &mut Appearance) -> Result<(), ConfigError> {
    let children = required_children(node)?;
    for child in children.nodes() {
        match child.name().value() {
            "background-color" => {
                appearance.background_color = parse_color(node_string(child)?)?;
            }
            "focus-indicator-width" => appearance.focus_indicator_width = node_u32(child)?,
            "focus-indicator-height" => appearance.focus_indicator_height = node_u32(child)?,
            "focus-indicator-color" => {
                appearance.focus_indicator_color = parse_color(node_string(child)?)?;
            }
            "window-border-width" => appearance.window_border_width = node_u32(child)?,
            "window-border-color" => {
                appearance.window_border_color = parse_color(node_string(child)?)?;
            }
            "corner-radius" => appearance.corner_radius = node_u32(child)?,
            "gaps" => appearance.gaps = node_u32(child)?,
            "opacity" => {
                #[allow(clippy::cast_possible_truncation)]
                let opacity = node_number(child)? as f32;
                appearance.opacity = opacity;
            }
            "opacity-toggle" => {
                #[allow(clippy::cast_possible_truncation)]
                let first = node_number_at(child, 0)? as f32;
                #[allow(clippy::cast_possible_truncation)]
                let second = node_number_at(child, 1)? as f32;
                appearance.opacity_toggle = [first, second];
            }
            name => {
                return Err(ConfigError::new(format!(
                    "unknown appearance option `{name}`"
                )))
            }
        }
    }
    Ok(())
}

fn parse_binding(node: &KdlNode) -> Result<KeyBinding, ConfigError> {
    let chord = KeyChord::from_str(node_string_at(node, 0)?)?;
    let action_name = node_string_at(node, 1)?;
    let action = if action_name == "camera-zoom" {
        if node.entries().len() != 3 {
            return Err(ConfigError::new(
                "camera-zoom bind expects exactly one zoom value",
            ));
        }
        let zoom = node_number_at(node, 2)?;
        if !(0.1..=1.0).contains(&zoom) {
            return Err(ConfigError::new(
                "camera-zoom bind value must be between 0.1 and 1.0",
            ));
        }
        ConfigAction::CameraZoom(zoom)
    } else if action_name == "spawn" {
        ConfigAction::Spawn(parse_argv(node, 2, "spawn bind")?)
    } else {
        if node.entries().len() != 2 {
            return Err(ConfigError::new(format!(
                "{action_name} bind does not accept additional arguments"
            )));
        }
        parse_action(action_name)?
    };
    Ok(KeyBinding { chord, action })
}

fn parse_window_rule(node: &KdlNode) -> Result<WindowRule, ConfigError> {
    let mut app_id = node
        .get("app-id")
        .and_then(|entry| entry.value().as_string())
        .map(str::to_owned);
    let mut title = node
        .get("title")
        .and_then(|entry| entry.value().as_string())
        .map(str::to_owned);
    let mut opacity = None;
    let mut floating = None;
    let mut blur = None;
    if let Some(children) = node.children() {
        for child in children.nodes() {
            match child.name().value() {
                "match" => {
                    app_id = optional_string_property(child, "app-id")?.or(app_id);
                    title = optional_string_property(child, "title")?.or(title);
                }
                "opacity" => {
                    #[allow(clippy::cast_possible_truncation)]
                    let value = node_number(child)? as f32;
                    opacity = Some(value);
                }
                "floating" => {
                    floating = Some(
                        child
                            .get(0)
                            .and_then(|entry| match entry.value() {
                                KdlValue::Bool(value) => Some(*value),
                                _ => None,
                            })
                            .ok_or_else(|| ConfigError::new("floating expects true or false"))?,
                    );
                }
                "blur" => {
                    blur = Some(node_bool(child)?);
                }
                name => {
                    return Err(ConfigError::new(format!(
                        "unknown window-rule option `{name}`"
                    )))
                }
            }
        }
    }
    if app_id.is_none() && title.is_none() {
        return Err(ConfigError::new(
            "window-rule requires an app-id or title matcher",
        ));
    }
    if opacity.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        return Err(ConfigError::new(
            "window-rule opacity must be between 0 and 1",
        ));
    }
    Ok(WindowRule {
        app_id,
        title,
        opacity,
        floating,
        blur,
    })
}

fn optional_string_property(node: &KdlNode, name: &str) -> Result<Option<String>, ConfigError> {
    node.get(name)
        .map(|entry| {
            entry
                .value()
                .as_string()
                .map(str::to_owned)
                .ok_or_else(|| ConfigError::new(format!("`{name}` expects a string")))
        })
        .transpose()
}

fn node_bool(node: &KdlNode) -> Result<bool, ConfigError> {
    node.get(0)
        .and_then(|entry| match entry.value() {
            KdlValue::Bool(value) => Some(*value),
            _ => None,
        })
        .ok_or_else(|| ConfigError::new(format!("`{}` expects true or false", node.name().value())))
}

fn node_window_transition(node: &KdlNode) -> Result<WindowTransitionEffect, ConfigError> {
    let Some(value) = node.get(0).map(kdl::KdlEntry::value) else {
        return Err(ConfigError::new(
            "window-transition expects \"water\", \"sci-fi\", \"none\", true, or false",
        ));
    };
    match value {
        KdlValue::Bool(true) => Ok(WindowTransitionEffect::Water),
        KdlValue::Bool(false) => Ok(WindowTransitionEffect::None),
        KdlValue::String(value) if value == "water" => Ok(WindowTransitionEffect::Water),
        KdlValue::String(value) if value == "sci-fi" => Ok(WindowTransitionEffect::SciFi),
        KdlValue::String(value) if value == "none" => Ok(WindowTransitionEffect::None),
        _ => Err(ConfigError::new(
            "window-transition expects \"water\", \"sci-fi\", \"none\", true, or false",
        )),
    }
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    if !config.output.scale.is_finite() || !(0.5..=4.0).contains(&config.output.scale) {
        return Err(ConfigError::new("output scale must be between 0.5 and 4.0"));
    }
    if !(0.0..=1.0).contains(&config.appearance.opacity) {
        return Err(ConfigError::new(
            "appearance opacity must be between 0 and 1",
        ));
    }
    if config
        .appearance
        .opacity_toggle
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(ConfigError::new(
            "appearance opacity-toggle values must be between 0 and 1",
        ));
    }
    if !config.effects.cursor_wake_threshold.is_finite()
        || !(1.0..=10_000.0).contains(&config.effects.cursor_wake_threshold)
    {
        return Err(ConfigError::new(
            "effects cursor-wake-threshold must be between 1 and 10000",
        ));
    }
    if !config.effects.cursor_wake_strength.is_finite()
        || !(0.0..=0.2).contains(&config.effects.cursor_wake_strength)
    {
        return Err(ConfigError::new(
            "effects cursor-wake-strength must be between 0 and 0.2",
        ));
    }
    if !config.effects.cursor_wake_width.is_finite()
        || !(1.0..=64.0).contains(&config.effects.cursor_wake_width)
    {
        return Err(ConfigError::new(
            "effects cursor-wake-width must be between 1 and 64",
        ));
    }
    if !(100..=10_000).contains(&config.effects.cursor_wake_duration) {
        return Err(ConfigError::new(
            "effects cursor-wake-duration must be between 100 and 10000 milliseconds",
        ));
    }
    if !(100..=5_000).contains(&config.effects.window_transition_duration) {
        return Err(ConfigError::new(
            "effects window-transition-duration must be between 100 and 5000 milliseconds",
        ));
    }
    if (config.appearance.opacity_toggle[0] - config.appearance.opacity_toggle[1]).abs()
        < f32::EPSILON
    {
        return Err(ConfigError::new(
            "appearance opacity-toggle values must be different",
        ));
    }
    if !config.animation_speed.is_finite() || config.animation_speed < 0.0 {
        return Err(ConfigError::new(
            "animation speed must be finite and non-negative",
        ));
    }
    if !(1..=8).contains(&config.effects.blur_passes) {
        return Err(ConfigError::new(
            "effects blur-passes must be between 1 and 8",
        ));
    }
    if !config.effects.blur_offset.is_finite()
        || !(0.5..=20.0).contains(&config.effects.blur_offset)
    {
        return Err(ConfigError::new(
            "effects blur-offset must be between 0.5 and 20",
        ));
    }
    if !config.effects.shadow_radius.is_finite()
        || !(0.0..=256.0).contains(&config.effects.shadow_radius)
    {
        return Err(ConfigError::new(
            "effects shadow-radius must be between 0 and 256",
        ));
    }
    if config
        .effects
        .shadow_offset
        .iter()
        .any(|offset| !(-4096..=4096).contains(offset))
    {
        return Err(ConfigError::new(
            "effects shadow-offset values must be between -4096 and 4096",
        ));
    }
    if config.appearance.corner_radius > 4096
        || config.appearance.gaps > 4096
        || config.appearance.focus_indicator_width > 4096
        || config.appearance.focus_indicator_height > 4096
        || config.appearance.window_border_width > 4096
    {
        return Err(ConfigError::new(
            "appearance dimensions must not exceed 4096",
        ));
    }
    validate_mouse(&config.mouse)?;
    Ok(())
}

fn required_children(node: &KdlNode) -> Result<&KdlDocument, ConfigError> {
    node.children().ok_or_else(|| {
        ConfigError::new(format!("`{}` requires a child block", node.name().value()))
    })
}

fn child_number(node: &KdlNode, name: &str) -> Result<f64, ConfigError> {
    let child = required_children(node)?
        .get(name)
        .ok_or_else(|| ConfigError::new(format!("`{}` requires `{name}`", node.name().value())))?;
    node_number(child)
}

fn child_size(node: &KdlNode, name: &str) -> Result<GridSize, ConfigError> {
    let child = required_children(node)?
        .get(name)
        .ok_or_else(|| ConfigError::new(format!("`{}` requires `{name}`", node.name().value())))?;
    let width = node_u64_at(child, 0)?;
    let height = node_u64_at(child, 1)?;
    GridSize::new(width, height)
        .map_err(|error| ConfigError::new(format!("invalid {name}: {error}")))
}

#[allow(clippy::cast_precision_loss)]
fn node_number(node: &KdlNode) -> Result<f64, ConfigError> {
    node_number_at(node, 0)
}

#[allow(clippy::cast_precision_loss)]
fn node_number_at(node: &KdlNode, index: usize) -> Result<f64, ConfigError> {
    node.get(index)
        .and_then(|entry| {
            entry
                .value()
                .as_f64()
                .or_else(|| entry.value().as_i64().map(|value| value as f64))
        })
        .ok_or_else(|| {
            ConfigError::new(format!(
                "`{}` expects a number at index {index}",
                node.name().value()
            ))
        })
}

fn node_u32(node: &KdlNode) -> Result<u32, ConfigError> {
    u32::try_from(node_u64_at(node, 0)?)
        .map_err(|_| ConfigError::new(format!("`{}` is too large", node.name().value())))
}

fn node_i32_at(node: &KdlNode, index: usize) -> Result<i32, ConfigError> {
    node.get(index)
        .and_then(|entry| entry.value().as_i64())
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            ConfigError::new(format!(
                "`{}` expects a 32-bit integer at index {index}",
                node.name().value()
            ))
        })
}

fn node_u64_at(node: &KdlNode, index: usize) -> Result<u64, ConfigError> {
    node.get(index)
        .and_then(|entry| entry.value().as_i64())
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            ConfigError::new(format!(
                "`{}` expects a non-negative integer at index {index}",
                node.name().value()
            ))
        })
}

fn node_u64_property(node: &KdlNode, name: &str) -> Result<u64, ConfigError> {
    node.get(name)
        .and_then(|entry| entry.value().as_i64())
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            ConfigError::new(format!(
                "`{}` requires a non-negative integer `{name}` property",
                node.name().value()
            ))
        })
}

fn node_string(node: &KdlNode) -> Result<&str, ConfigError> {
    node_string_at(node, 0)
}

fn node_string_at(node: &KdlNode, index: usize) -> Result<&str, ConfigError> {
    node.get(index)
        .and_then(|entry| entry.value().as_string())
        .ok_or_else(|| {
            ConfigError::new(format!(
                "`{}` expects a string at index {index}",
                node.name().value()
            ))
        })
}

fn parse_color(value: &str) -> Result<[f32; 4], ConfigError> {
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| ConfigError::new("color must start with #"))?;
    if hex.len() != 6 && hex.len() != 8 {
        return Err(ConfigError::new("color must use #RRGGBB or #RRGGBBAA"));
    }
    let channel = |offset| {
        u8::from_str_radix(&hex[offset..offset + 2], 16).map(|value| f32::from(value) / 255.0)
    };
    Ok([
        channel(0).map_err(|_| ConfigError::new("invalid red color channel"))?,
        channel(2).map_err(|_| ConfigError::new("invalid green color channel"))?,
        channel(4).map_err(|_| ConfigError::new("invalid blue color channel"))?,
        if hex.len() == 8 {
            channel(6).map_err(|_| ConfigError::new("invalid alpha color channel"))?
        } else {
            1.0
        },
    ])
}

impl FromStr for KeyChord {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts = value.split('+').collect::<Vec<_>>();
        let (key_name, modifiers) = parts
            .split_last()
            .ok_or_else(|| ConfigError::new("empty key chord"))?;
        let mut chord = Self {
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
            key: parse_key(key_name)?,
        };
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "super" | "logo" => chord.logo = true,
                _ => return Err(ConfigError::new(format!("unknown modifier `{modifier}`"))),
            }
        }
        Ok(chord)
    }
}

fn parse_key(value: &str) -> Result<Key, ConfigError> {
    let lowercase = value.to_ascii_lowercase();
    match lowercase.as_str() {
        "left" => Ok(Key::Left),
        "right" => Ok(Key::Right),
        "up" => Ok(Key::Up),
        "down" => Ok(Key::Down),
        "enter" | "return" => Ok(Key::Enter),
        _ if value.chars().count() == 1 => {
            let character = value.chars().next().expect("one character");
            Ok(key_from_keysym_raw(
                xkb::utf32_to_keysym(character.to_ascii_lowercase() as u32).raw(),
            ))
        }
        _ => parse_named_keysym(value),
    }
}

fn parse_named_keysym(value: &str) -> Result<Key, ConfigError> {
    if value.contains('\0') {
        return Err(ConfigError::new("key name cannot contain NUL"));
    }
    let alias = match value.to_ascii_lowercase().as_str() {
        "esc" => "Escape",
        "spacebar" => "space",
        "printscreen" | "prtsc" | "prtscr" => "Print",
        "pageup" | "pgup" => "Page_Up",
        "pagedown" | "pgdown" | "pgdn" => "Page_Down",
        "backspace" => "BackSpace",
        "capslock" => "Caps_Lock",
        "numlock" => "Num_Lock",
        "scrolllock" => "Scroll_Lock",
        _ => value,
    };
    let mut symbol = xkb::keysym_from_name(alias, xkb::KEYSYM_NO_FLAGS);
    if symbol.raw() == 0 {
        symbol = xkb::keysym_from_name(alias, xkb::KEYSYM_CASE_INSENSITIVE);
    }
    if symbol.raw() == 0 {
        Err(ConfigError::new(format!("unsupported key `{value}`")))
    } else {
        Ok(key_from_keysym_raw(symbol.raw()))
    }
}

pub(crate) fn key_from_keysym_raw(raw: u32) -> Key {
    match raw {
        value if value == xkb::keysyms::KEY_Left => Key::Left,
        value if value == xkb::keysyms::KEY_Right => Key::Right,
        value if value == xkb::keysyms::KEY_Up => Key::Up,
        value if value == xkb::keysyms::KEY_Down => Key::Down,
        value if value == xkb::keysyms::KEY_Return => Key::Enter,
        value => {
            let unicode = xkb::keysym_to_utf32(xkb::Keysym::new(value));
            (unicode != 0)
                .then(|| char::from_u32(unicode))
                .flatten()
                .map_or(Key::Symbol(value), |character| {
                    Key::Letter(character.to_ascii_lowercase())
                })
        }
    }
}

fn parse_action(value: &str) -> Result<ConfigAction, ConfigError> {
    let direction = |prefix: &str| value.strip_prefix(prefix).and_then(parse_direction);
    if let Some(direction) = direction("focus-") {
        return Ok(ConfigAction::Focus(direction));
    }
    if let Some(direction) = direction("camera-") {
        return Ok(ConfigAction::Camera(direction));
    }
    if let Some(direction) = direction("camera-nudge-") {
        return Ok(ConfigAction::CameraNudge(direction));
    }
    if let Some(direction) = direction("move-") {
        return Ok(ConfigAction::Move(direction));
    }
    if let Some(direction) = direction("resize-") {
        return Ok(ConfigAction::Resize(direction));
    }
    if let Some(direction) = direction("place-next-") {
        return Ok(ConfigAction::PlaceNext(direction));
    }
    match value {
        "close" => Ok(ConfigAction::Close),
        "cycle-output" => Ok(ConfigAction::CycleOutput),
        "toggle-floating" => Ok(ConfigAction::ToggleFloating),
        "toggle-fullscreen" => Ok(ConfigAction::ToggleFullscreen),
        "toggle-maximized" => Ok(ConfigAction::ToggleMaximized),
        "toggle-window-size" => Ok(ConfigAction::ToggleWindowSize),
        "toggle-opacity" => Ok(ConfigAction::ToggleOpacity),
        "toggle-blur" => Ok(ConfigAction::ToggleBlur),
        "toggle-cursor-wake" => Ok(ConfigAction::ToggleCursorWake),
        "clear-opacity" => Ok(ConfigAction::ClearOpacity),
        "toggle-overview" => Ok(ConfigAction::ToggleOverview),
        "select-overview" => Ok(ConfigAction::SelectOverview),
        "reload-config" => Ok(ConfigAction::ReloadConfig),
        _ => Err(ConfigError::new(format!("unknown action `{value}`"))),
    }
}

fn parse_direction(value: &str) -> Option<Direction> {
    match value {
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    }
}

fn default_bindings() -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    let mut push = |chord: &str, action| {
        bindings.push(KeyBinding {
            chord: chord.parse().expect("default chord is valid"),
            action,
        });
    };
    for (key, direction) in [
        ("Left", Direction::Left),
        ("Right", Direction::Right),
        ("Up", Direction::Up),
        ("Down", Direction::Down),
    ] {
        push(&format!("Super+{key}"), ConfigAction::Focus(direction));
        push(
            &format!("Super+Ctrl+{key}"),
            ConfigAction::Camera(direction),
        );
        push(&format!("Super+Shift+{key}"), ConfigAction::Move(direction));
        push(
            &format!("Super+Ctrl+Shift+{key}"),
            ConfigAction::Resize(direction),
        );
    }
    for (key, zoom) in [
        ('1', 0.1),
        ('2', 0.2),
        ('3', 0.3),
        ('4', 0.4),
        ('5', 0.5),
        ('6', 0.6),
        ('7', 0.7),
        ('8', 0.8),
        ('9', 0.9),
        ('0', 1.0),
    ] {
        push(&format!("Super+{key}"), ConfigAction::CameraZoom(zoom));
    }
    for (key, direction) in [
        ("H", Direction::Left),
        ("L", Direction::Right),
        ("K", Direction::Up),
        ("J", Direction::Down),
    ] {
        push(&format!("Super+{key}"), ConfigAction::Focus(direction));
        push(
            &format!("Super+Ctrl+{key}"),
            ConfigAction::Camera(direction),
        );
        push(
            &format!("Super+Ctrl+Shift+{key}"),
            ConfigAction::CameraNudge(direction),
        );
        push(
            &format!("Super+Shift+{key}"),
            ConfigAction::PlaceNext(direction),
        );
    }
    push("Super+F", ConfigAction::ToggleFloating);
    push("Super+N", ConfigAction::CycleOutput);
    push("Super+Enter", ConfigAction::ToggleFullscreen);
    push("Super+M", ConfigAction::ToggleMaximized);
    push("Super+Z", ConfigAction::ToggleWindowSize);
    push("Super+Q", ConfigAction::Close);
    push("Super+O", ConfigAction::ToggleOpacity);
    push("Super+B", ConfigAction::ToggleBlur);
    push("Super+W", ConfigAction::ToggleCursorWake);
    push("Super+Shift+O", ConfigAction::ClearOpacity);
    push("Super+V", ConfigAction::ToggleOverview);
    push("Super+S", ConfigAction::SelectOverview);
    push("Super+R", ConfigAction::ReloadConfig);
    bindings
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ConfigError {}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::too_many_lines)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mio-{name}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn defaults_new_windows_to_the_full_viewport() {
        let config = Config::default();
        assert!((config.output.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.initial_window_size, config.viewport);
        assert_eq!(config.viewport, GridSize::new(8, 8).unwrap());
        assert_eq!(config.appearance.gaps, 24);
        assert_eq!(config.appearance.background_color, [1.0; 4]);
        assert_eq!(config.effects.shadow_offset, [0, 0]);
        assert_eq!(config.appearance.window_border_width, 1);
        assert_eq!(
            config.appearance.window_border_color,
            [1.0, 1.0, 1.0, 46.0 / 255.0]
        );
        assert_eq!(config.mouse.close_window_clicks, 3);
    }

    #[test]
    fn example_keybindings_match_the_builtin_defaults() {
        let example = parse(include_str!("../../../config/mio.kdl")).unwrap();
        let defaults = default_bindings();
        assert_eq!(example.bindings.len(), defaults.len());
        assert!(defaults
            .iter()
            .all(|binding| example.bindings.contains(binding)));
    }

    #[test]
    fn parses_complete_configuration() {
        let config = parse(
            r##"
            output {
                scale 1.25
            }
            appearance {
                background-color "#102030ff"
                window-border-width 2
                window-border-color "#abcdef40"
                focus-indicator-width 80
                focus-indicator-height 2
                focus-indicator-color "#336699cc"
                corner-radius 10
                gaps 12
                opacity 0.8
                opacity-toggle 1.0 0.7
            }
            effects {
                blur-passes 4
                blur-offset 2.5
                shadow-radius 30
                shadow-offset -4 12
                shadow-color "#11223380"
                cursor-wake false
                cursor-wake-threshold 1200
                cursor-wake-strength 0.025
                cursor-wake-width 7.0
                cursor-wake-duration 900
                window-transition false
                window-transition-duration 800
            }
            animation {
                speed 1.25
            }
            camera {
                viewport 5 3
            }
            placement {
                initial-size 2 1
            }
            mouse {
                cursor-hide-delay-ms 2500
                camera-pan "middle"
                camera-zoom "middle"
                move-window "middle" "right"
                resize-window "left"
                reset-window "middle" clicks=2
                toggle-floating "middle" "left"
                place-next "right"
                center-window "middle" "right" clicks=1
                close-window "middle" "right" clicks=4
            }
            spawn-at-startup "waybar"
            spawn-at-startup "kaname" "--applications"
            edge-command "left" "foot" "--title=edge terminal"
            bind "Alt+H" "focus-left"
            window-rule {
                match app-id="foot" title="terminal"
                opacity 0.9
                floating true
                blur true
            }
        "##,
        )
        .unwrap();
        assert!((config.output.scale - 1.25).abs() < f64::EPSILON);
        assert_eq!(config.appearance.focus_indicator_width, 80);
        assert_eq!(config.appearance.focus_indicator_height, 2);
        assert_eq!(
            config.appearance.background_color,
            parse_color("#102030ff").unwrap()
        );
        assert_eq!(config.appearance.window_border_width, 2);
        let expected_border_color = parse_color("#abcdef40").unwrap();
        assert!(config
            .appearance
            .window_border_color
            .iter()
            .zip(expected_border_color)
            .all(|(actual, expected)| (*actual - expected).abs() < f32::EPSILON));
        assert_eq!(config.appearance.gaps, 12);
        assert!((config.appearance.opacity - 0.8).abs() < f32::EPSILON);
        assert!((config.appearance.opacity_toggle[0] - 1.0).abs() < f32::EPSILON);
        assert!((config.appearance.opacity_toggle[1] - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.effects.blur_passes, 4);
        assert!((config.effects.blur_offset - 2.5).abs() < f32::EPSILON);
        assert!((config.effects.shadow_radius - 30.0).abs() < f32::EPSILON);
        assert_eq!(config.effects.shadow_offset, [-4, 12]);
        let expected_shadow_color = parse_color("#11223380").unwrap();
        assert!(config
            .effects
            .shadow_color
            .iter()
            .zip(expected_shadow_color)
            .all(|(actual, expected)| (*actual - expected).abs() < f32::EPSILON));
        assert!(!config.effects.cursor_wake);
        assert!((config.effects.cursor_wake_threshold - 1200.0).abs() < f32::EPSILON);
        assert!((config.effects.cursor_wake_strength - 0.025).abs() < f32::EPSILON);
        assert!((config.effects.cursor_wake_width - 7.0).abs() < f32::EPSILON);
        assert_eq!(config.effects.cursor_wake_duration, 900);
        assert_eq!(
            config.effects.window_transition,
            WindowTransitionEffect::None
        );
        assert_eq!(config.effects.window_transition_duration, 800);
        assert_eq!(
            config.startup_commands,
            [
                vec!["waybar".to_owned()],
                vec!["kaname".to_owned(), "--applications".to_owned()]
            ]
        );
        assert_eq!(config.viewport, GridSize::new(5, 3).unwrap());
        assert_eq!(config.edge_commands.len(), 1);
        assert_eq!(config.mouse.camera_pan, MouseButton::Middle);
        assert_eq!(config.mouse.cursor_hide_delay_ms, 2500);
        assert_eq!(config.mouse.reset_window_clicks, 2);
        assert_eq!(
            config.mouse.move_window,
            [MouseButton::Middle, MouseButton::Right]
        );
        assert_eq!(
            config.mouse.close_window,
            [MouseButton::Middle, MouseButton::Right]
        );
        assert_eq!(config.mouse.close_window_clicks, 4);
        assert_eq!(config.mouse.center_window_clicks, 1);
        assert_eq!(config.edge_commands[0].edge, Direction::Left);
        assert_eq!(
            config.edge_commands[0].argv,
            ["foot", "--title=edge terminal"]
        );
        assert_eq!(config.bindings.len(), 1);
        assert_eq!(config.window_rules[0].app_id.as_deref(), Some("foot"));
        assert_eq!(config.window_rules[0].title.as_deref(), Some("terminal"));
        assert_eq!(config.window_rules[0].blur, Some(true));
    }

    #[test]
    fn rejects_invalid_values_and_duplicate_bindings() {
        assert!(parse("appearance {\n opacity 1.2\n}").is_err());
        assert!(parse("output {\n scale 0.0\n}").is_err());
        assert!(parse("output {\n scale 4.1\n}").is_err());
        assert!(parse("output {\n unknown 1.0\n}").is_err());
        assert!(parse("appearance {\n opacity-toggle 1.0 1.0\n}").is_err());
        assert!(parse("appearance {\n opacity-toggle 1.0 1.2\n}").is_err());
        assert!(parse("appearance {\n gaps 4097\n}").is_err());
        assert!(parse("appearance {\n window-border-width 4097\n}").is_err());
        assert!(parse("bind \"Alt+H\" \"focus-left\"\nbind \"Alt+H\" \"focus-right\"").is_err());
        assert!(parse("mouse { move-window \"left\" \"left\" }").is_err());
        assert!(parse("mouse { close-window \"right\" \"middle\" }").is_err());
        assert!(parse("mouse { close-window \"right\" \"left\" }").is_err());
        assert!(parse("mouse { close-window \"right\" \"left\" clicks=0 }").is_err());
        assert!(parse("mouse { close-window \"right\" \"left\" clicks=6 }").is_err());
        assert!(parse("mouse { camera-pan \"side\" }").is_err());
        assert!(parse("mouse { reset-window \"middle\" }").is_err());
        assert!(parse("mouse { center-window \"right\" \"left\" }").is_err());
        assert!(parse(
            r#"
            mouse {
                center-window "right" "left" clicks=3
                close-window "right" "left" clicks=3
            }
            "#,
        )
        .is_err());
        assert!(parse("unknown true").is_err());
        assert!(parse("spawn-at-startup").is_err());
        assert!(parse("spawn-at-startup \"\"").is_err());
        assert!(parse("spawn-at-startup \"foot\" 1").is_err());
        assert!(parse("edge-command \"middle\" \"foot\"").is_err());
        assert!(parse("edge-command \"left\"").is_err());
        assert!(parse("edge-command \"left\" \"foot\"\nedge-command \"left\" \"wofi\"").is_err());
        assert!(parse("effects {\n blur-passes 0\n}").is_err());
        assert!(parse("effects {\n blur-offset 30\n}").is_err());
        assert!(parse("effects {\n shadow-radius -1\n}").is_err());
        assert!(parse("effects {\n shadow-radius 257\n}").is_err());
        assert!(parse("effects {\n shadow-offset 5000 0\n}").is_err());
        assert!(parse("effects {\n cursor-wake-threshold 0\n}").is_err());
        assert!(parse("effects {\n cursor-wake-strength 0.3\n}").is_err());
        assert!(parse("effects {\n cursor-wake-width 0\n}").is_err());
        assert!(parse("effects {\n cursor-wake-duration 50\n}").is_err());
        assert!(parse("effects {\n window-transition-duration 50\n}").is_err());
        let syntax_error = parse("appearance {\n opacity =\n}").unwrap_err();
        assert!(syntax_error.to_string().contains("line 2"));
    }

    #[test]
    fn parses_one_shot_placement_actions() {
        assert_eq!(
            parse_action("place-next-left").unwrap(),
            ConfigAction::PlaceNext(Direction::Left)
        );
        assert_eq!(
            parse_action("place-next-down").unwrap(),
            ConfigAction::PlaceNext(Direction::Down)
        );
    }

    #[test]
    fn parses_blur_toggle_action() {
        assert_eq!(
            parse_action("toggle-blur").unwrap(),
            ConfigAction::ToggleBlur
        );
    }

    #[test]
    fn parses_window_size_toggle_action() {
        assert_eq!(
            parse_action("toggle-window-size").unwrap(),
            ConfigAction::ToggleWindowSize
        );
    }

    #[test]
    fn parses_absolute_camera_zoom_bindings() {
        let config = parse(
            "bind \"Super+5\" \"camera-zoom\" 0.55\n\
             bind \"Super+0\" \"camera-zoom\" 1.0",
        )
        .unwrap();
        assert_eq!(config.bindings.len(), 2);
        assert_eq!(config.bindings[0].action, ConfigAction::CameraZoom(0.55));
        assert_eq!(config.bindings[1].action, ConfigAction::CameraZoom(1.0));
        assert!(parse("bind \"Super+1\" \"camera-zoom\" 0.09").is_err());
        assert!(parse("bind \"Super+0\" \"camera-zoom\" 1.01").is_err());
        assert!(parse("bind \"Super+5\" \"camera-zoom\"").is_err());
        assert!(parse("bind \"Super+5\" \"camera-zoom\" 0.5 0.6").is_err());
    }

    #[test]
    fn parses_external_command_binding_as_argv() {
        let config = parse(r#"bind "Super+W" "spawn" "bash" "-c" "do something""#).unwrap();
        assert_eq!(
            config.bindings[0].action,
            ConfigAction::Spawn(vec!["bash".into(), "-c".into(), "do something".into()])
        );
        assert!(parse(r#"bind "Super+W" "spawn""#).is_err());
        assert!(parse(r#"bind "Super+Q" "close" "unexpected""#).is_err());
    }

    #[test]
    fn parses_named_xkb_keys_and_common_aliases() {
        let space = KeyChord::from_str("Super+Space").unwrap();
        assert_eq!(space.key, Key::Letter(' '));

        let print = KeyChord::from_str("PrintScreen").unwrap();
        assert_eq!(
            print.key,
            Key::Symbol(xkb::keysym_from_name("Print", xkb::KEYSYM_NO_FLAGS).raw())
        );

        for name in [
            "Escape",
            "Tab",
            "Delete",
            "Home",
            "PageDown",
            "F12",
            "XF86AudioRaiseVolume",
        ] {
            assert!(KeyChord::from_str(name).is_ok(), "failed to parse {name}");
        }
        assert!(KeyChord::from_str("DefinitelyNotARealKey").is_err());
    }

    #[test]
    fn loads_relative_includes_in_place_and_ignores_missing_files() {
        let directory = test_directory("config-include");
        fs::create_dir_all(&directory).unwrap();
        let main = directory.join("config.kdl");
        let wallpaper = directory.join("wallpaper.kdl");
        fs::write(
            &main,
            "appearance {\n opacity 0.7\n}\ninclude \"missing.kdl\"\ninclude \"wallpaper.kdl\"\nappearance {\n opacity 0.9\n}\n",
        )
        .unwrap();
        fs::write(
            &wallpaper,
            "spawn-at-startup \"mpvpaper\" \"*\" \"wallpaper.mp4\"\nappearance {\n opacity 0.8\n}\n",
        )
        .unwrap();

        let config = parse_file(&main).unwrap();
        assert_eq!(
            config.startup_commands,
            [vec![
                "mpvpaper".to_owned(),
                "*".to_owned(),
                "wallpaper.mp4".to_owned()
            ]]
        );
        assert!((config.appearance.opacity - 0.9).abs() < f32::EPSILON);

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resolves_relative_includes_next_to_a_symlinked_main_config() {
        use std::os::unix::fs::symlink;

        let directory = test_directory("config-symlink-include");
        let store = directory.join("store");
        let visible = directory.join("visible");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(&visible).unwrap();
        let stored_main = store.join("config.kdl");
        let visible_main = visible.join("config.kdl");
        fs::write(&stored_main, "include \"wallpaper.kdl\"\n").unwrap();
        fs::write(
            visible.join("wallpaper.kdl"),
            "spawn-at-startup \"awww-daemon\"\n",
        )
        .unwrap();
        symlink(&stored_main, &visible_main).unwrap();

        let config = parse_file(&visible_main).unwrap();
        assert_eq!(config.startup_commands, [vec!["awww-daemon".to_owned()]]);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reports_included_file_errors_and_include_cycles() {
        let directory = test_directory("config-include-errors");
        fs::create_dir_all(&directory).unwrap();
        let main = directory.join("config.kdl");
        let child = directory.join("child.kdl");
        fs::write(&main, "include \"child.kdl\"\n").unwrap();
        fs::write(&child, "appearance {\n opacity 4.0\n}\n").unwrap();

        let error = parse_file(&main).unwrap_err().to_string();
        assert!(error.contains(&child.display().to_string()));
        assert!(error.contains("line 2"));
        assert!(error.contains("opacity 4.0"));

        fs::write(&child, "include \"config.kdl\"\n").unwrap();
        let error = parse_file(&main).unwrap_err().to_string();
        assert!(error.contains("configuration include cycle"));
        assert!(error.contains(&main.display().to_string()));
        assert!(error.contains(&child.display().to_string()));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn include_requires_a_file_context_and_one_string_path() {
        assert!(parse("include \"wallpaper.kdl\"").is_err());

        let directory = test_directory("config-invalid-include");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.kdl");
        fs::write(&path, "include \"one.kdl\" \"two.kdl\"\n").unwrap();
        assert!(parse_file(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_cursor_wake_toggle_action() {
        assert_eq!(
            parse_action("toggle-cursor-wake").unwrap(),
            ConfigAction::ToggleCursorWake
        );
    }

    #[test]
    fn parses_window_transition_styles_and_boolean_compatibility() {
        let sci_fi = parse("effects {\n window-transition \"sci-fi\"\n}").unwrap();
        assert_eq!(
            sci_fi.effects.window_transition,
            WindowTransitionEffect::SciFi
        );
        let water = parse("effects {\n window-transition true\n}").unwrap();
        assert_eq!(
            water.effects.window_transition,
            WindowTransitionEffect::Water
        );
        assert!(parse("effects {\n window-transition \"warp\"\n}").is_err());
    }

    #[test]
    fn failed_reload_keeps_last_valid_configuration() {
        let directory = test_directory("config-reload");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.kdl");
        fs::write(&path, "appearance {\n opacity 0.7\n}").unwrap();
        let mut manager = ConfigManager::load(Some(path.clone()), true).unwrap();
        fs::write(&path, "appearance {\n opacity 4.0\n}").unwrap();
        assert!(manager.reload().is_err());
        assert!((manager.config().appearance.opacity - 0.7).abs() < f32::EPSILON);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_startup_load_uses_defaults_and_retains_path_for_reload() {
        let directory = test_directory("config-fallback");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.kdl");
        fs::write(&path, "appearance {\n opacity 4.0\n}").unwrap();

        let (mut manager, error) = ConfigManager::load_or_default(Some(path.clone()), true);
        let error = error.unwrap().to_string();
        assert!(error.contains("line 2"));
        assert!(error.contains("opacity 4.0"));
        assert_eq!(manager.config(), &Config::default());
        assert_eq!(manager.display_path(), path.display().to_string());

        fs::write(&path, "appearance {\n opacity 0.7\n}").unwrap();
        manager.reload().unwrap();
        assert!((manager.config().appearance.opacity - 0.7).abs() < f32::EPSILON);
        fs::remove_dir_all(directory).unwrap();
    }
}
