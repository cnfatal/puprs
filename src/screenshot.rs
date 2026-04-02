/// Image format for screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

/// Options for taking a screenshot.
#[derive(Debug, Clone, Default)]
pub struct ScreenshotOptions {
    pub format: ImageFormat,
    /// JPEG/WebP quality (0–100). Ignored for PNG.
    pub quality: Option<i64>,
    /// If set, capture only this region (x, y, width, height, scale).
    pub clip: Option<ClipRect>,
    /// Capture beyond the viewport.
    pub capture_beyond_viewport: Option<bool>,
    /// Capture from the surface rather than the view.
    pub from_surface: Option<bool>,
}

/// A rectangular clip region.
#[derive(Debug, Clone, Copy)]
pub struct ClipRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl ClipRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale: 1.0,
        }
    }
}

impl From<ScreenshotOptions>
    for chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams
{
    fn from(o: ScreenshotOptions) -> Self {
        use chromiumoxide::cdp::browser_protocol::page::{
            CaptureScreenshotFormat, CaptureScreenshotParams, Viewport,
        };

        let mut params = CaptureScreenshotParams::default();
        params.format = Some(match o.format {
            ImageFormat::Png => CaptureScreenshotFormat::Png,
            ImageFormat::Jpeg => CaptureScreenshotFormat::Jpeg,
            ImageFormat::Webp => CaptureScreenshotFormat::Webp,
        });
        params.quality = o.quality;
        params.capture_beyond_viewport = o.capture_beyond_viewport;
        params.from_surface = o.from_surface;

        if let Some(clip) = o.clip {
            params.clip = Some(Viewport {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            });
        }

        params
    }
}

/// Options for generating a PDF.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    pub landscape: Option<bool>,
    pub display_header_footer: Option<bool>,
    pub print_background: Option<bool>,
    pub scale: Option<f64>,
    pub paper_width: Option<f64>,
    pub paper_height: Option<f64>,
    pub margin_top: Option<f64>,
    pub margin_bottom: Option<f64>,
    pub margin_left: Option<f64>,
    pub margin_right: Option<f64>,
    pub page_ranges: Option<String>,
    pub header_template: Option<String>,
    pub footer_template: Option<String>,
    pub prefer_css_page_size: Option<bool>,
}

impl From<PdfOptions> for chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams {
    fn from(o: PdfOptions) -> Self {
        let mut p = chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams::default();
        p.landscape = o.landscape;
        p.display_header_footer = o.display_header_footer;
        p.print_background = o.print_background;
        p.scale = o.scale;
        p.paper_width = o.paper_width;
        p.paper_height = o.paper_height;
        p.margin_top = o.margin_top;
        p.margin_bottom = o.margin_bottom;
        p.margin_left = o.margin_left;
        p.margin_right = o.margin_right;
        p.page_ranges = o.page_ranges;
        p.header_template = o.header_template;
        p.footer_template = o.footer_template;
        p.prefer_css_page_size = o.prefer_css_page_size;
        p
    }
}
