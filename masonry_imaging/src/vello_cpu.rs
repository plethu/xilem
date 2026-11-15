// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use core::fmt;

/// Errors that can occur while creating or using a Vello CPU renderer.
#[derive(Debug)]
pub enum Error {
    /// The underlying `imaging_vello_cpu` renderer failed.
    Backend(imaging_vello_cpu::Error),
    /// Rasterising a prepared frame failed.
    Render(imaging::render::ImageRendererError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(err) => write!(f, "Vello CPU render failed: {err:?}"),
            Self::Render(err) => write!(f, "Vello CPU frame render failed: {err:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Stable backend name for diagnostics.
pub const BACKEND_NAME: &str = "imaging_vello_cpu";

/// Masonry alias for the selected Vello CPU renderer type.
pub type Renderer = imaging_vello_cpu::VelloCpuRenderer;

/// Create a reusable headless Vello CPU renderer.
pub fn new_headless_renderer() -> Result<Renderer, Error> {
    Ok(imaging_vello_cpu::VelloCpuRenderer::new(1, 1))
}

/// Rasterise a prepared window frame into tightly packed, unpremultiplied RGBA8 pixels.
///
/// `pixels` is reused across frames and resized to the frame's dimensions.
pub fn render_rgba8(
    renderer: &mut Renderer,
    frame: crate::PreparedFrame<'_>,
    pixels: &mut Vec<u8>,
) -> Result<(), Error> {
    use imaging::render::{ImageBufferFormat, ImageBufferTarget, ImageRenderer};

    let (width, height) = (frame.width, frame.height);
    let bytes_per_row = width as usize * 4;
    pixels.resize(bytes_per_row * height as usize, 0);
    let mut frame = frame;
    renderer
        .render_source_into(
            &mut frame,
            ImageBufferTarget {
                data: pixels,
                width,
                height,
                bytes_per_row,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .map_err(Error::Render)
}

/// Rasterise one rectangle of a prepared window frame.
///
/// `pixels` receives the whole frame as tightly packed, unpremultiplied RGBA8; only `rect` holds
/// frame content and the rest is transparent. Geometry outside the rectangle is clipped before
/// fine rasterisation.
///
/// Geometry, text and solid colours match [`render_rgba8`] exactly. Image and gradient pixels
/// match to within one unit per channel when the rectangle is aligned to Vello CPU's 256 × 4 wide
/// tiles (or ends at the frame's edge): the clip routes some of their spans through Vello CPU's
/// masked path, which rounds differently. Unaligned rectangles can also shift image samples.
pub fn render_region_rgba8(
    renderer: &mut Renderer,
    frame: crate::PreparedFrame<'_>,
    rect: crate::PixelRect,
    pixels: &mut Vec<u8>,
) -> Result<(), Error> {
    use imaging::render::{ImageBufferFormat, ImageBufferTarget, ImageRenderer};

    let (width, height) = (frame.width, frame.height);
    let bytes_per_row = width as usize * 4;
    pixels.resize(bytes_per_row * height as usize, 0);
    let mut region = crate::FrameRegion { frame, rect };
    renderer
        .render_source_into(
            &mut region,
            ImageBufferTarget {
                data: pixels,
                width,
                height,
                bytes_per_row,
                format: ImageBufferFormat::Rgba8Unorm,
            },
        )
        .map_err(Error::Render)
}

/// Pack an opaque RGBA8 pixel as `0x00RRGGBB`.
#[inline]
pub fn pack_xrgb(rgba: &[u8]) -> u32 {
    (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2])
}

/// Renders window frames into a persistent `0x00RRGGBB` framebuffer, re-rasterising only damaged
/// rectangles.
///
/// Each damaged rectangle is widened to Vello CPU's wide tiles and rendered by clipping the whole
/// frame to it, then copied into the framebuffer. The result matches a full render as described
/// for [`render_region_rgba8`]: exactly, except for one-unit rounding in image and gradient pixels. Pixels outside the damage keep
/// their previous values, so the caller must report every change since the previous call;
/// [`invalidate`](Self::invalidate) forces the next frame to be rendered in full.
#[derive(Debug)]
pub struct IncrementalRenderer {
    renderer: Renderer,
    scratch: Vec<u8>,
    framebuffer: Vec<u32>,
    width: u32,
    height: u32,
    background: Option<peniko::Color>,
    scale_factor: f64,
    valid: bool,
}

impl IncrementalRenderer {
    /// Create a renderer with an empty framebuffer.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            renderer: new_headless_renderer()?,
            scratch: Vec::new(),
            framebuffer: Vec::new(),
            width: 0,
            height: 0,
            background: None,
            scale_factor: 0.0,
            valid: false,
        })
    }

    /// Render the next frame in full on the next call to [`render`](Self::render).
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// The framebuffer as `0x00RRGGBB` pixels, row-major and tightly packed.
    pub fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }

    /// Framebuffer dimensions.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Render a frame into the framebuffer.
    ///
    /// `damage` lists the window-space rectangles, in logical pixels, that changed since the
    /// previous frame; `None` means the whole frame changed. The frame is rendered in full when the
    /// size, scale factor or background colour changed, or after an error or
    /// [`invalidate`](Self::invalidate).
    ///
    /// Returns the physical rectangles that were updated, or `None` when the whole frame was.
    pub fn render(
        &mut self,
        frame: crate::PreparedFrame<'_>,
        damage: Option<&[kurbo::Rect]>,
    ) -> Result<Option<Vec<crate::PixelRect>>, Error> {
        let (width, height) = (frame.width, frame.height);
        let full = !self.valid
            || self.width != width
            || self.height != height
            || self.scale_factor != frame.scale_factor
            || self.background != Some(frame.background_color);
        self.width = width;
        self.height = height;
        self.scale_factor = frame.scale_factor;
        self.background = Some(frame.background_color);
        // An early return leaves the framebuffer partly stale.
        self.valid = false;

        let rects = match damage {
            Some(damage) if !full => Some(crate::damage_to_pixel_rects(
                damage,
                frame.scale_factor,
                (width, height),
                DAMAGE_MARGIN,
                REGION_ALIGN,
                MAX_DAMAGE_RECTS,
            )),
            _ => None,
        };
        let window = crate::PixelRect::new(0, 0, width, height);
        // Rendering a region costs about as much scene work as a full frame, so a region that
        // covers most of the window is rendered in full.
        let rects = rects.filter(|rects| {
            rects.iter().map(crate::PixelRect::area).sum::<u64>() * 4 <= window.area() * 3
        });
        let Some(rects) = rects else {
            render_rgba8(&mut self.renderer, frame, &mut self.scratch)?;
            self.framebuffer.resize(width as usize * height as usize, 0);
            for (pixel, rgba) in self
                .framebuffer
                .iter_mut()
                .zip(self.scratch.chunks_exact(4))
            {
                *pixel = pack_xrgb(rgba);
            }
            self.valid = true;
            return Ok(None);
        };

        for rect in &rects {
            render_region_rgba8(&mut self.renderer, frame, *rect, &mut self.scratch)?;
            for row in rect.y..rect.bottom() {
                let start = row as usize * width as usize + rect.x as usize;
                let src = &self.scratch[start * 4..(start + rect.width as usize) * 4];
                let dst = &mut self.framebuffer[start..start + rect.width as usize];
                for (pixel, rgba) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    *pixel = pack_xrgb(rgba);
                }
            }
        }
        self.valid = true;
        Ok(Some(rects))
    }
}

/// Physical pixels added around each damaged rectangle, covering antialiasing.
const DAMAGE_MARGIN: u32 = 2;
/// Damaged rectangles rendered separately per frame; more are merged.
const MAX_DAMAGE_RECTS: usize = 4;
/// Region alignment: Vello CPU's wide tile is 256 × 4 pixels. Images and gradients are sampled
/// incrementally along each tile's spans, so a clip edge inside a wide tile shifts their samples by
/// rounding error, which flips nearest-neighbour image samples at fractional scale factors.
const REGION_ALIGN: (u32, u32) = (256, 4);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use imaging::Painter;
    use imaging::record::Scene;
    use kurbo::{Affine, Circle, Rect, RoundedRect, Stroke};
    use peniko::{
        Blob, Color, Extend, Gradient, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
        ImageQuality,
    };

    use super::{IncrementalRenderer, new_headless_renderer, render_region_rgba8, render_rgba8};
    use crate::{PixelRect, PreparedFrame};

    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 400;
    const BACKGROUND: Color = Color::from_rgb8(240, 236, 228);

    fn grain() -> ImageBrush {
        let (w, h) = (13_u32, 7_u32);
        let pixels = (0..w * h)
            .flat_map(|i| {
                let alpha = u8::try_from((i * 37 + 11) % 256).unwrap();
                [40, 30, 20, alpha]
            })
            .collect::<Vec<_>>();
        ImageBrush::new(ImageData {
            data: Blob::new(Arc::new(pixels)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: w,
            height: h,
        })
        .with_extend(Extend::Repeat)
        .with_quality(ImageQuality::Low)
    }

    /// A scene with the kinds of content Writer paints; `step` moves some of it.
    fn scene(grain: &ImageBrush, step: u32) -> Scene {
        let mut scene = Scene::new();
        let mut painter = Painter::new(&mut scene);
        painter
            .fill(Rect::new(0.0, 0.0, 400.0, 400.0), grain)
            .draw();
        let gradient = Gradient::new_linear((10.0, 10.0), (190.0, 120.0)).with_stops([
            Color::from_rgb8(200, 40, 40),
            Color::from_rgba8(40, 40, 200, 128),
        ]);
        painter
            .fill(RoundedRect::new(12.3, 15.7, 180.2, 110.9, 7.5), &gradient)
            .draw();
        painter
            .stroke(
                Circle::new((150.5, 100.25), 40.0 + f64::from(step)),
                &Stroke::new(2.5),
                Color::from_rgb8(10, 120, 30),
            )
            .draw();
        let x = 20.0 + f64::from(step) * 7.3;
        painter.fill_rect(
            Rect::new(x, 130.4, x + 1.5, 150.6),
            Color::from_rgb8(0, 0, 0),
        );
        painter
            .fill(Rect::new(200.0, 20.0, 280.0, 180.0), grain)
            .transform(Affine::rotate(0.1))
            .draw();
        scene
    }

    fn full(frame: PreparedFrame<'_>) -> Vec<u32> {
        let mut renderer = new_headless_renderer().unwrap();
        let mut pixels = Vec::new();
        render_rgba8(&mut renderer, frame, &mut pixels).unwrap();
        pixels.chunks_exact(4).map(super::pack_xrgb).collect()
    }

    /// A scene without images or gradients; `step` moves some of it.
    fn solid_scene(step: u32) -> Scene {
        let mut scene = Scene::new();
        let mut painter = Painter::new(&mut scene);
        painter.fill_rect(
            Rect::new(5.5, 5.5, 290.25, 190.75),
            Color::from_rgba8(30, 60, 90, 60),
        );
        painter
            .fill(
                RoundedRect::new(12.3, 15.7, 180.2 + f64::from(step), 110.9, 7.5),
                Color::from_rgb8(200, 40, 40),
            )
            .draw();
        painter
            .stroke(
                Circle::new((150.5, 100.25), 40.0 + f64::from(step)),
                &Stroke::new(2.5),
                Color::from_rgba8(10, 120, 30, 200),
            )
            .draw();
        let x = 20.0 + f64::from(step) * 7.3;
        painter.fill_rect(
            Rect::new(x, 130.4, x + 1.5, 150.6),
            Color::from_rgb8(0, 0, 0),
        );
        scene
    }

    fn assert_close(actual: &[u32], expected: &[u32], tolerance: u32, context: &str) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
            let close = [0, 8, 16]
                .into_iter()
                .all(|shift| ((a >> shift) & 0xff).abs_diff((b >> shift) & 0xff) <= tolerance);
            assert!(
                close,
                "{context}: pixel ({}, {}) is {a:06x}, expected {b:06x}",
                i % WIDTH as usize,
                i / WIDTH as usize
            );
        }
    }

    #[test]
    fn aligned_regions_match_the_full_frame() {
        let grain = grain();
        for (name, base, tolerance) in [
            ("solid", solid_scene(0), 0),
            ("images and gradients", scene(&grain, 0), 1),
        ] {
            for scale in [1.0, 1.25, 2.0] {
                let frame = PreparedFrame::new(WIDTH, HEIGHT, scale, BACKGROUND, &base, &[]);
                let expected = full(frame);
                let mut renderer = new_headless_renderer().unwrap();
                let mut pixels = Vec::new();
                for rect in [
                    PixelRect::new(0, 0, 256, 32),
                    PixelRect::new(0, 24, 256, 40),
                    PixelRect::new(256, 96, 384, 104),
                    PixelRect::new(512, 200, 128, 200),
                    PixelRect::new(0, 4, 640, 100),
                ] {
                    render_region_rgba8(&mut renderer, frame, rect, &mut pixels).unwrap();
                    let packed: Vec<u32> = pixels.chunks_exact(4).map(super::pack_xrgb).collect();
                    for row in rect.y..rect.bottom() {
                        let span =
                            (row * WIDTH + rect.x) as usize..(row * WIDTH + rect.right()) as usize;
                        assert_close(
                            &packed[span.clone()],
                            &expected[span],
                            tolerance,
                            &format!("{name} at scale {scale} in {rect:?}"),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn incremental_frames_match_full_frames() {
        let grain = grain();
        for (name, tolerance) in [("solid", 0), ("images and gradients", 1)] {
            let mut incremental = IncrementalRenderer::new().unwrap();
            for step in 0..6 {
                let base = if tolerance == 0 {
                    solid_scene(step)
                } else {
                    scene(&grain, step)
                };
                let frame = PreparedFrame::new(WIDTH, HEIGHT, 1.25, BACKGROUND, &base, &[]);
                let previous = f64::from(step.saturating_sub(1));
                let current = f64::from(step);
                // The moving caret, the growing circle and the growing rounded rectangle, in
                // logical pixels.
                let damage = [
                    Rect::new(20.0 + previous * 7.3, 130.4, 21.5 + current * 7.3, 150.6),
                    Rect::new(
                        150.5 - 42.0 - current,
                        100.25 - 42.0 - current,
                        150.5 + 42.0 + current,
                        100.25 + 42.0 + current,
                    ),
                    Rect::new(12.3, 15.7, 180.2 + current, 110.9),
                ];
                let updated = incremental
                    .render(frame, (step > 0).then_some(&damage[..]))
                    .unwrap();
                assert_eq!(updated.is_none(), step == 0, "{name} step {step}");
                if let Some(rects) = &updated {
                    let area: u64 = rects.iter().map(PixelRect::area).sum();
                    assert!(
                        area < u64::from(WIDTH * HEIGHT),
                        "{name} step {step}: {rects:?}"
                    );
                }
                assert_close(
                    incremental.framebuffer(),
                    &full(frame),
                    tolerance,
                    &format!("{name} step {step}"),
                );
            }
        }
    }
}
