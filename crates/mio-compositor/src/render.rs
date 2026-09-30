use smithay::{
    backend::renderer::{
        element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
        gles::GlesRenderer,
    },
    desktop::{layer_map_for_output, LayerSurface},
    output::Output,
    utils::{Rectangle, Scale},
    wayland::shell::wlr_layer::Layer as WlrLayer,
};

pub(crate) fn upper_layer_element_count(renderer: &mut GlesRenderer, output: &Output) -> usize {
    let map = layer_map_for_output(output);
    let output_scale = output.current_scale().fractional_scale();
    map.layers()
        .rev()
        .filter(|layer| matches!(layer.layer(), WlrLayer::Top | WlrLayer::Overlay))
        .filter_map(|layer| map.layer_geometry(layer).map(|geometry| (layer, geometry)))
        .map(|(layer, geometry)| {
            <LayerSurface as AsRenderElements<GlesRenderer>>::render_elements::<
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(
                layer,
                renderer,
                geometry.loc.to_physical_precise_round(output_scale),
                Scale::from(output_scale),
                1.0,
            )
            .len()
        })
        .sum()
}

pub(crate) fn config_error_text_rectangles(
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    height: i32,
    error: &str,
) -> Vec<Rectangle<i32, smithay::utils::Physical>> {
    bitmap_text_rects("CONFIG ERROR - USING DEFAULTS", (14, 8), 2)
        .into_iter()
        .chain(bitmap_text_rects(
            "FIX FILE, THEN RELOAD CONFIG",
            (14, 28),
            2,
        ))
        .chain(bitmap_text_rects(
            &config_error_summary(
                error,
                usize::try_from((size.w - 28).max(0) / 12).unwrap_or(0),
            ),
            (14, 48),
            2,
        ))
        .filter(|rectangle| rectangle.loc.x < size.w && rectangle.loc.y < height)
        .collect()
}

fn bitmap_text_rects(
    text: &str,
    origin: (i32, i32),
    scale: i32,
) -> Vec<Rectangle<i32, smithay::utils::Physical>> {
    let mut rectangles = Vec::new();
    for (character_index, character) in text.chars().enumerate() {
        let Ok(character_index) = i32::try_from(character_index) else {
            break;
        };
        let x = origin
            .0
            .saturating_add(character_index.saturating_mul(6).saturating_mul(scale));
        for (row, bits) in glyph_rows(character).into_iter().enumerate() {
            let Ok(row) = i32::try_from(row) else {
                continue;
            };
            let mut column = 0_i32;
            while column < 5 {
                if bits & (1 << (4 - column)) == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < 5 && bits & (1 << (4 - column)) != 0 {
                    column += 1;
                }
                rectangles.push(Rectangle::new(
                    (
                        x.saturating_add(start.saturating_mul(scale)),
                        origin.1.saturating_add(row.saturating_mul(scale)),
                    )
                        .into(),
                    ((column - start).saturating_mul(scale), scale).into(),
                ));
            }
        }
    }
    rectangles
}

#[allow(clippy::too_many_lines)]
fn glyph_rows(character: char) -> [u8; 7] {
    match character {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        ',' => [0, 0, 0, 0, 0, 0b00100, 0b01000],
        '.' => [0, 0, 0, 0, 0, 0, 0b00100],
        ':' => [0, 0b00100, 0, 0, 0b00100, 0, 0],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        _ => [0; 7],
    }
}

fn config_error_summary(error: &str, max_chars: usize) -> String {
    let detail = error.split_once(": ").map_or(error, |(_, detail)| detail);
    detail
        .chars()
        .map(|character| {
            let character = character.to_ascii_uppercase();
            if character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '+' | '-' | ',' | '.' | ':' | '/' | '_')
            {
                character
            } else {
                ' '
            }
        })
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{bitmap_text_rects, config_error_summary, glyph_rows};

    #[test]
    fn configuration_error_text_has_visible_pixels() {
        assert_ne!(glyph_rows('A'), [0; 7]);
        assert_eq!(glyph_rows(' '), [0; 7]);
        let rectangles = bitmap_text_rects("CONFIG ERROR", (14, 8), 2);
        assert!(!rectangles.is_empty());
        assert!(rectangles.iter().all(|rectangle| {
            rectangle.loc.x >= 14
                && rectangle.loc.y >= 8
                && rectangle.size.w > 0
                && rectangle.size.h > 0
        }));
        assert_eq!(
            config_error_summary(
                "/tmp/mio.kdl: appearance opacity must be between 0 and 1 at line 2",
                80
            ),
            "APPEARANCE OPACITY MUST BE BETWEEN 0 AND 1 AT LINE 2"
        );
    }
}
