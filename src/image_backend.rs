//! Real-graphics piece rendering via `ratatui-image` (Sixel / Kitty / iTerm2,
//! with a half-block fallback inside the crate). Compiled only with the
//! `images` Cargo feature.
//!
//! The piece artwork is embedded from `assets/` at build time. On startup the
//! terminal is queried for a supported graphics protocol; each piece image is
//! decoded once, and a protocol is built and cached per cell size on first use.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;

use image::DynamicImage;
use image::Rgba;
use image::RgbaImage;
use miette::miette;
use miette::Result;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::FilterType;
use ratatui_image::Image;
use ratatui_image::Resize;

/// The ascii board characters that map to a piece image.
const PIECE_CHARACTERS: [char; 12] = ['K', 'Q', 'R', 'B', 'N', 'P', 'k', 'q', 'r', 'b', 'n', 'p'];

/// The embedded PNG for an ascii board character (uppercase = white).
fn piece_png(board_character: char) -> Option<&'static [u8]> {
    let bytes: &'static [u8] = match board_character {
        'K' => include_bytes!("../assets/wK.png"),
        'Q' => include_bytes!("../assets/wQ.png"),
        'R' => include_bytes!("../assets/wR.png"),
        'B' => include_bytes!("../assets/wB.png"),
        'N' => include_bytes!("../assets/wN.png"),
        'P' => include_bytes!("../assets/wP.png"),
        'k' => include_bytes!("../assets/bK.png"),
        'q' => include_bytes!("../assets/bQ.png"),
        'r' => include_bytes!("../assets/bR.png"),
        'b' => include_bytes!("../assets/bB.png"),
        'n' => include_bytes!("../assets/bN.png"),
        'p' => include_bytes!("../assets/bP.png"),
        _ => return None,
    };
    Some(bytes)
}

/// Holds the graphics protocol picker, the decoded piece images, and a cache of
/// per-cell-size protocols.
pub struct ImageBackend {
    picker: Picker,
    decoded: HashMap<char, RgbaImage>,
    /// Cached protocols keyed by (piece character, cell width, cell height).
    protocols: RefCell<HashMap<(char, u16, u16), Protocol>>,
    /// The cell size the cache was built for. Every square shares one size, so
    /// when it changes the old protocols are dead weight and get dropped —
    /// otherwise a single resize drag would leave hundreds of stale images.
    cached_cell_size: Cell<(u16, u16)>,
}

impl std::fmt::Debug for ImageBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageBackend")
            .field("pieces", &self.decoded.len())
            .finish_non_exhaustive()
    }
}

impl ImageBackend {
    /// Queries the terminal for a graphics protocol and decodes the artwork.
    /// Errors if no protocol could be detected.
    pub fn new() -> Result<Self> {
        let picker = Picker::from_query_stdio()
            .map_err(|error| miette!("no terminal graphics protocol available: {error}"))?;
        Self::from_picker(picker)
    }

    /// Builds a backend from an already-configured picker, decoding the artwork.
    fn from_picker(picker: Picker) -> Result<Self> {
        let mut decoded = HashMap::new();
        for character in PIECE_CHARACTERS {
            if let Some(bytes) = piece_png(character) {
                let image = image::load_from_memory(bytes)
                    .map_err(|error| miette!("could not decode piece image: {error}"))?;
                decoded.insert(character, image.to_rgba8());
            }
        }

        Ok(ImageBackend {
            picker,
            decoded,
            protocols: RefCell::new(HashMap::new()),
            cached_cell_size: Cell::new((0, 0)),
        })
    }

    /// Centers the piece on a transparent canvas whose aspect matches the cell.
    ///
    /// This is what makes centering exact: handing a square image straight to
    /// `Resize::Fit` leaves a fitted area that rarely matches the cell in whole
    /// cells, so the piece hugs one side. Matching the cell's aspect first means
    /// the fitted area fills the cell and the piece sits centered within it at
    /// pixel precision.
    ///
    /// The canvas is built at the *source's* resolution and the piece is copied
    /// into it unscaled, so this costs an allocation and a copy rather than a
    /// resample. Scaling down to the cell is left to `new_protocol`, which has
    /// to resize anyway — doing a high-quality resize here as well made every
    /// terminal resize noticeably slow.
    fn canvas_for_cell(&self, board_character: char, cell: Rect) -> Option<DynamicImage> {
        let source = self.decoded.get(&board_character)?;
        let (font_width, font_height) = self.picker.font_size();
        let cell_pixel_width = u32::from(cell.width) * u32::from(font_width);
        let cell_pixel_height = u32::from(cell.height) * u32::from(font_height);
        if cell_pixel_width == 0 || cell_pixel_height == 0 {
            return None;
        }

        let source_width = source.width().max(1);
        let source_height = source.height().max(1);
        let cell_aspect = f64::from(cell_pixel_width) / f64::from(cell_pixel_height);

        // Grow the canvas (never crop) until its aspect matches the cell.
        let widened = (f64::from(source_height) * cell_aspect).round() as u32;
        let (canvas_width, canvas_height) = if widened >= source_width {
            (widened, source_height)
        } else {
            (
                source_width,
                (f64::from(source_width) / cell_aspect).round() as u32,
            )
        };
        let canvas_width = canvas_width.max(source_width);
        let canvas_height = canvas_height.max(source_height);

        // The board keeps its squares square, so the cell's aspect is usually
        // already the source's and no padding is needed at all.
        if canvas_width == source_width && canvas_height == source_height {
            return Some(DynamicImage::ImageRgba8(source.clone()));
        }

        let mut canvas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba([0, 0, 0, 0]));
        image::imageops::overlay(
            &mut canvas,
            source,
            i64::from((canvas_width - source_width) / 2),
            i64::from((canvas_height - source_height) / 2),
        );
        Some(DynamicImage::ImageRgba8(canvas))
    }

    /// The terminal's font size in pixels, as `(width, height)`. The board
    /// layout uses this to keep squares square.
    pub fn font_size(&self) -> (u16, u16) {
        self.picker.font_size()
    }

    /// Renders the piece for `board_character` into `cell`, building and caching
    /// the protocol for this cell size on first use.
    pub fn render(&self, frame: &mut Frame<'_>, cell: Rect, board_character: char) {
        if cell.width == 0 || cell.height == 0 {
            return;
        }
        let key = (board_character, cell.width, cell.height);
        let mut protocols = self.protocols.borrow_mut();

        if self.cached_cell_size.get() != (cell.width, cell.height) {
            protocols.clear();
            self.cached_cell_size.set((cell.width, cell.height));
        }

        if let std::collections::hash_map::Entry::Vacant(entry) = protocols.entry(key) {
            let Some(canvas) = self.canvas_for_cell(board_character, cell) else {
                return;
            };
            let size = Rect::new(0, 0, cell.width, cell.height);
            // The artwork is much larger than a cell, so this is the one place a
            // real downscale happens. Ask for Lanczos3 explicitly: the default
            // filter resamples coarsely and leaves the pieces looking grainy.
            match self
                .picker
                .new_protocol(canvas, size, Resize::Fit(Some(FilterType::Lanczos3)))
            {
                Ok(protocol) => {
                    entry.insert(protocol);
                }
                Err(_) => return,
            }
        }

        if let Some(protocol) = protocols.get(&key) {
            // `Fit` scales the (square) image proportionally, so the fitted area
            // is usually smaller than the cell in one dimension. Center it in
            // the cell rather than letting it hug the top-left corner.
            let fitted = protocol.area();
            let offset_x = cell.width.saturating_sub(fitted.width) / 2;
            let offset_y = cell.height.saturating_sub(fitted.height) / 2;
            let placed = Rect::new(
                cell.x + offset_x,
                cell.y + offset_y,
                fitted.width.min(cell.width),
                fitted.height.min(cell.height),
            );
            frame.render_widget(Image::new(protocol), placed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    /// Renders a piece through the real ratatui-image pipeline using the
    /// half-block protocol (no graphics terminal needed) and checks that the
    /// decode -> protocol -> widget path actually paints coloured cells.
    #[test]
    fn renders_piece_via_halfblocks() {
        let backend = ImageBackend::from_picker(Picker::halfblocks())
            .expect("decoding the embedded artwork should succeed");

        let mut terminal = Terminal::new(TestBackend::new(16, 8)).unwrap();
        terminal
            .draw(|frame| backend.render(frame, ratatui::layout::Rect::new(0, 0, 10, 6), 'K'))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut painted = 0;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = buffer.cell((x, y)).unwrap();
                let has_color = cell.fg != Color::Reset || cell.bg != Color::Reset;
                if cell.symbol() != " " || has_color {
                    painted += 1;
                }
            }
        }
        assert!(
            painted > 0,
            "the king image should paint cells; got {painted}"
        );
    }

    /// The canvas fills the cell exactly, and the piece's ink is centered within
    /// it at pixel precision, for cells of any width/height parity. This is the
    /// property that keeps pieces centered on their squares.
    #[test]
    fn piece_ink_is_centered_in_the_canvas() {
        let backend = ImageBackend::from_picker(Picker::halfblocks()).unwrap();
        for (width, height) in [(7u16, 3u16), (8, 3), (9, 4), (11, 5), (7, 4)] {
            let canvas = backend
                .canvas_for_cell('K', Rect::new(0, 0, width, height))
                .expect("canvas should build");
            let pixels = canvas.to_rgba8();
            let (canvas_width, canvas_height) = (pixels.width(), pixels.height());

            let (font_width, font_height) = backend.picker.font_size();
            let cell_pixel_width = u32::from(width) * u32::from(font_width);
            let cell_pixel_height = u32::from(height) * u32::from(font_height);
            // The canvas is built at the source's resolution, not the cell's,
            // but its aspect must match the cell so that fitting it fills the
            // cell exactly and leaves the piece centered.
            let canvas_aspect = f64::from(canvas_width) / f64::from(canvas_height);
            let cell_aspect = f64::from(cell_pixel_width) / f64::from(cell_pixel_height);
            assert!(
                (canvas_aspect - cell_aspect).abs() < 0.02,
                "canvas aspect {canvas_aspect:.3} should match cell aspect {cell_aspect:.3} \
                 at {width}x{height}"
            );

            let mut min_x = u32::MAX;
            let mut max_x = 0;
            let mut min_y = u32::MAX;
            let mut max_y = 0;
            for y in 0..canvas_height {
                for x in 0..canvas_width {
                    if pixels.get_pixel(x, y)[3] > 8 {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            let left = i64::from(min_x);
            let right = i64::from(canvas_width - 1 - max_x);
            let top = i64::from(min_y);
            let bottom = i64::from(canvas_height - 1 - max_y);
            assert!(
                (left - right).abs() <= 1 && (top - bottom).abs() <= 1,
                "piece off-center in {width}x{height}: L={left} R={right} T={top} B={bottom}"
            );
        }
    }

    #[test]
    fn decodes_all_twelve_pieces() {
        let backend = ImageBackend::from_picker(Picker::halfblocks()).unwrap();
        assert_eq!(backend.decoded.len(), 12);
    }
}
