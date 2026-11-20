//! The bear (spec, "Look and layout"): the main bear, rasterised once from the
//! committed SVG (`assets/brand/bear-mark.svg`) and drawn with the kitty graphics
//! protocol where the terminal answers to it, half-blocks elsewhere.
//!
//! Each placement (header, splash, empty state) keeps its own protocol state: a
//! resized image is tied to one area, so sharing one would re-encode on every
//! frame. `ui::frame` skips any placement an overlay covers, so a sheet never has
//! bear pixels over it.

use image::{DynamicImage, Rgba, RgbaImage, imageops};
use ratatui::{Frame, layout::Rect, style::Color};
use ratatui_image::{
    FontSize, Resize, StatefulImage,
    picker::{Picker, ProtocolType, cap_parser::QueryStdioOptions},
    protocol::StatefulProtocol,
};
use raudbjorn_tui::profile::{ColorProfile, TerminalProfile};
use resvg::{tiny_skia, usvg};
use std::sync::OnceLock;

/// The main bear, embedded so den needs no asset files at run time.
const BEAR_SVG: &[u8] = include_bytes!("../../assets/brand/bear-mark.svg");
/// Longest edge of the raster, in pixels. The splash bear is at most
/// [`SPLASH_BEAR_MAX_ROWS`] rows (about 350 px in a 22 px font); the rest is headroom.
const RASTER_EDGE_PX: f32 = 768.0;
/// How long to wait for the terminal to answer the graphics query, in milliseconds.
const GRAPHICS_QUERY_TIMEOUT_MS: i32 = 1000;
/// Overrides graphics detection: `kitty`, `halfblocks` or `off`.
pub const GRAPHICS_ENV: &str = "DEN_GRAPHICS";
/// Height of the splash bear at most, in rows.
pub const SPLASH_BEAR_MAX_ROWS: u16 = 16;
/// Height of the empty-state bear at most, in rows.
pub const EMPTY_BEAR_MAX_ROWS: u16 = 12;
/// The empty-state bear is left out below this height, in rows.
pub const EMPTY_BEAR_MIN_ROWS: u16 = 3;

/// How the bear is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Graphics {
    /// The kitty graphics protocol (unicode placeholders).
    Kitty,
    /// Coloured half-block characters, two pixels per cell.
    Halfblocks,
    /// No image: the brand text stands alone.
    Off,
}

impl Graphics {
    /// Kitty when the terminal answered to it; else half-blocks, which are
    /// coloured text and so need a true-colour profile; else no image.
    /// `requested` (from [`GRAPHICS_ENV`]) wins when it is set and valid.
    pub fn choose(
        detected: Option<ProtocolType>,
        requested: Option<Self>,
        profile: TerminalProfile,
    ) -> Self {
        if let Some(requested) = requested {
            return requested;
        }
        if detected == Some(ProtocolType::Kitty) {
            return Self::Kitty;
        }
        Self::halfblocks_or_off(profile)
    }

    /// Half-blocks when the colour profile can show them, else no image.
    pub fn halfblocks_or_off(profile: TerminalProfile) -> Self {
        match profile.color {
            ColorProfile::TrueColor => Self::Halfblocks,
            ColorProfile::Ansi256
            | ColorProfile::Ansi16
            | ColorProfile::Ansi8
            | ColorProfile::Mono
            | ColorProfile::NoColor => Self::Off,
        }
    }

    /// Parses a [`GRAPHICS_ENV`] value.
    pub fn parse(value: &str) -> Result<Self, BearError> {
        match value.trim() {
            "kitty" => Ok(Self::Kitty),
            "halfblocks" => Ok(Self::Halfblocks),
            "off" => Ok(Self::Off),
            other => Err(BearError::Env(other.to_owned())),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Halfblocks => "halfblocks",
            Self::Off => "off",
        }
    }
}

/// Where a bear sits. Each has its own protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Header,
    Splash,
    Empty,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BearError {
    #[error("{GRAPHICS_ENV} must be kitty, halfblocks or off, got {0:?}")]
    Env(String),
    #[error("the bear SVG could not be read: {0}")]
    Svg(String),
    #[error("the bear could not be rasterised: {0}")]
    Raster(&'static str),
}

/// One placement's protocol state, for the background it was composited on.
struct PlacementState {
    placement: Placement,
    background: [u8; 3],
    protocol: StatefulProtocol,
}

/// The bear's rendering state: the graphics mode, the terminal's font size and
/// one protocol state per placement.
pub struct BearArt {
    graphics: Graphics,
    picker: Picker,
    states: Vec<PlacementState>,
}

impl BearArt {
    /// Asks the terminal what it can draw. Call it after entering the alternate
    /// screen and before reading any input: the answer arrives on stdin.
    /// A failed query is not an error: den falls back to half-blocks.
    pub fn detect(profile: TerminalProfile, requested: Option<Graphics>) -> Self {
        let options = QueryStdioOptions {
            timeout_ms: GRAPHICS_QUERY_TIMEOUT_MS,
            ..QueryStdioOptions::default()
        };
        let (picker, detected) = match Picker::from_query_stdio_with_options(options) {
            Ok(picker) => {
                let detected = picker.protocol_type();
                (picker, Some(detected))
            }
            Err(error) => {
                tracing::info!(%error, "terminal did not answer the graphics query");
                (Picker::halfblocks(), None)
            }
        };
        let graphics = Graphics::choose(detected, requested, profile);
        tracing::info!(
            detected = ?detected,
            requested = requested.map(Graphics::label),
            graphics = graphics.label(),
            font_px = ?(picker.font_size().width, picker.font_size().height),
            "bear graphics chosen"
        );
        Self::with_picker(picker, graphics)
    }

    /// Half-blocks (or no image, if the profile has too few colours), without
    /// asking the terminal: for off-screen rendering.
    pub fn offscreen(profile: TerminalProfile) -> Self {
        Self::with_picker(Picker::halfblocks(), Graphics::halfblocks_or_off(profile))
    }

    fn with_picker(mut picker: Picker, graphics: Graphics) -> Self {
        let protocol = match graphics {
            Graphics::Kitty => ProtocolType::Kitty,
            Graphics::Halfblocks | Graphics::Off => ProtocolType::Halfblocks,
        };
        picker.set_protocol_type(protocol);
        Self {
            graphics,
            picker,
            states: Vec::new(),
        }
    }

    pub fn graphics(&self) -> Graphics {
        self.graphics
    }

    /// The largest bear that fits in `area` at most `max_rows` tall, centred.
    /// Empty when no image is drawn.
    pub fn fit(&self, area: Rect, max_rows: u16) -> Rect {
        if self.graphics == Graphics::Off {
            return Rect::default();
        }
        let rows = area.height.min(max_rows);
        let cols = self.cols_for_rows(rows).min(area.width);
        Rect::new(
            area.x + (area.width - cols) / 2,
            area.y + (area.height - rows) / 2,
            cols,
            rows,
        )
    }

    /// Columns a bear `rows` tall takes, given the font's cell shape.
    pub fn cols_for_rows(&self, rows: u16) -> u16 {
        if self.graphics == Graphics::Off {
            return 0;
        }
        let FontSize { width, height } = self.picker.font_size();
        let cell_aspect = f32::from(height) / f32::from(width.max(1));
        let cols = (f32::from(rows) * cell_aspect * bear_aspect()).round();
        // Bounded by u16 rows times small factors; the cast cannot overflow in practice.
        cols.clamp(0.0, f32::from(u16::MAX)) as u16
    }

    /// Draws the bear for `placement` into `area`; half-blocks are composited on
    /// `background` (the plate's true-colour role), kitty images are transparent.
    /// If the bear cannot be rasterised, den logs why once and goes on without
    /// it: a missing picture never takes the screen down.
    pub fn draw(&mut self, frame: &mut Frame, placement: Placement, area: Rect, background: Color) {
        if self.graphics == Graphics::Off || area.is_empty() {
            return;
        }
        match self.state_for(placement, rgb(background)) {
            Ok(protocol) => {
                let widget = StatefulImage::default().resize(Resize::Fit(None));
                frame.render_stateful_widget(widget, area, protocol);
            }
            Err(error) => {
                tracing::error!(%error, ?placement, "the bear is not drawn");
                self.graphics = Graphics::Off;
            }
        }
    }

    /// The protocol state of one placement, made on first use.
    fn state_for(
        &mut self,
        placement: Placement,
        background: [u8; 3],
    ) -> Result<&mut StatefulProtocol, BearError> {
        let existing = self
            .states
            .iter()
            .position(|s| s.placement == placement && s.background == background);
        let index = match existing {
            Some(index) => index,
            None => {
                let mut picker = self.picker.clone();
                let image = match self.graphics {
                    // kitty blends the image over whatever the cells' background
                    // is, so the plate shows through in every colour profile.
                    Graphics::Kitty => DynamicImage::ImageRgba8(raster()?.clone()),
                    // Half-blocks are solid: the bear and the padding that rounds
                    // it to whole cells both sit on the plate's colour.
                    Graphics::Halfblocks | Graphics::Off => {
                        let [r, g, b] = background;
                        picker.set_background_color(Some(Rgba([r, g, b, u8::MAX])));
                        on_background(raster()?, background)
                    }
                };
                self.states.push(PlacementState {
                    placement,
                    background,
                    protocol: picker.new_resize_protocol(image),
                });
                self.states.len() - 1
            }
        };
        self.states
            .get_mut(index)
            .map(|s| &mut s.protocol)
            .ok_or(BearError::Raster("placement state vanished"))
    }
}

/// Width over height of the bear's SVG canvas (1448×1086).
fn bear_aspect() -> f32 {
    const CANVAS_WIDTH_PX: f32 = 1448.0;
    const CANVAS_HEIGHT_PX: f32 = 1086.0;
    CANVAS_WIDTH_PX / CANVAS_HEIGHT_PX
}

/// The bear on a transparent canvas, rasterised once per process.
fn raster() -> Result<&'static RgbaImage, BearError> {
    static RASTER: OnceLock<Result<RgbaImage, BearError>> = OnceLock::new();
    RASTER.get_or_init(rasterise).as_ref().map_err(Clone::clone)
}

fn rasterise() -> Result<RgbaImage, BearError> {
    let tree = usvg::Tree::from_data(BEAR_SVG, &usvg::Options::default())
        .map_err(|e| BearError::Svg(e.to_string()))?;
    let size = tree.size();
    let scale = RASTER_EDGE_PX / size.width().max(size.height());
    let to_px = |edge: f32| (edge * scale).ceil().clamp(1.0, RASTER_EDGE_PX) as u32;
    let (width, height) = (to_px(size.width()), to_px(size.height()));
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or(BearError::Raster("zero-size canvas"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    RgbaImage::from_raw(width, height, rgba).ok_or(BearError::Raster("buffer size mismatch"))
}

/// The bear composited onto an opaque plate colour, for half-blocks (which
/// have no transparency: every cell gets two solid colours).
fn on_background(bear: &RgbaImage, [r, g, b]: [u8; 3]) -> DynamicImage {
    const OPAQUE: u8 = u8::MAX;
    let mut canvas = RgbaImage::from_pixel(bear.width(), bear.height(), Rgba([r, g, b, OPAQUE]));
    imageops::overlay(&mut canvas, bear, 0, 0);
    DynamicImage::ImageRgba8(canvas)
}

/// The RGB of a plate colour. Callers pass the true-colour palette's role (an
/// image is not quantised by the colour profile); anything else is black.
fn rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        _ => [0, 0, 0],
    }
}
