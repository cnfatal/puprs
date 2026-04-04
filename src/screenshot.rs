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

impl From<ScreenshotOptions> for crate::cdp::browser_protocol::page::CaptureScreenshotParams {
    fn from(o: ScreenshotOptions) -> Self {
        use crate::cdp::browser_protocol::page::{
            CaptureScreenshotFormat, CaptureScreenshotParams, Viewport,
        };

        CaptureScreenshotParams {
            format: Some(match o.format {
                ImageFormat::Png => CaptureScreenshotFormat::Png,
                ImageFormat::Jpeg => CaptureScreenshotFormat::Jpeg,
                ImageFormat::Webp => CaptureScreenshotFormat::Webp,
            }),
            quality: o.quality,
            clip: o.clip.map(|clip| Viewport {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            }),
            capture_beyond_viewport: o.capture_beyond_viewport,
            from_surface: o.from_surface,
            ..Default::default()
        }
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

impl From<PdfOptions> for crate::cdp::browser_protocol::page::PrintToPdfParams {
    fn from(o: PdfOptions) -> Self {
        crate::cdp::browser_protocol::page::PrintToPdfParams {
            landscape: o.landscape,
            display_header_footer: o.display_header_footer,
            print_background: o.print_background,
            scale: o.scale,
            paper_width: o.paper_width,
            paper_height: o.paper_height,
            margin_top: o.margin_top,
            margin_bottom: o.margin_bottom,
            margin_left: o.margin_left,
            margin_right: o.margin_right,
            page_ranges: o.page_ranges,
            header_template: o.header_template,
            footer_template: o.footer_template,
            prefer_css_page_size: o.prefer_css_page_size,
            ..Default::default()
        }
    }
}
