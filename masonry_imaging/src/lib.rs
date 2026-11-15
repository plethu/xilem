// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/linebender/xilem/main/docs/assets/masonry-logo.svg"
)]
//! Imaging helpers owned by Masonry.
//!
//! `masonry_imaging` owns the bridge between Masonry paint output and concrete imaging backends.
//! In this first slice that means:
//!
//! - preparing flattened Masonry frame content from base content plus overlays
//! - exposing backend-specific renderer modules for `imaging_vello`,
//!   `imaging_vello_hybrid`, `imaging_vello_cpu`, and `imaging_skia`
//! - exposing host-neutral texture rendering helpers for writing into caller-provided WGPU
//!   targets
//!
//! This crate does not own window integration, surfaces, or compositor policy.
//!
//! # Feature flags
//!
//! - `default`: Enables the `vello` module.
//! - `imaging_vello`: Enables the `vello` module and texture rendering support.
//! - `imaging_vello_hybrid`: Enables the `vello_hybrid` module and texture rendering support.
//! - `imaging_vello_cpu`: Enables the `vello_cpu` module for headless image rendering only.
//! - `imaging_skia`: Enables the `skia` module and texture rendering support on non-wasm targets.

// LINEBENDER LINT SET - lib.rs - v3
// See https://linebender.org/wiki/canonical-lints/
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![warn(clippy::print_stdout, clippy::print_stderr)]
#![cfg_attr(target_pointer_width = "64", warn(clippy::trivially_copy_pass_by_ref))]
// END LINEBENDER LINT SET
#![cfg_attr(docsrs, feature(doc_cfg))]

use imaging::record::{Scene, ValidateError, replay_transformed};
use imaging::render::RenderSource;
use imaging::{PaintSink, Painter};
use kurbo::{Affine, Rect};
use peniko::Color;

mod cull;
#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "imaging_vello", feature = "imaging_vello_hybrid")
))]
mod headless_wgpu;

/// Masonry helpers for rendering retained scenes with `imaging_skia`.
#[cfg(all(feature = "imaging_skia", not(target_arch = "wasm32")))]
pub mod skia;
/// Host-neutral texture rendering helpers for texture-capable backends.
#[cfg(not(target_arch = "wasm32"))]
pub mod texture_render;
/// Masonry helpers for rendering retained scenes with `imaging_vello`.
#[cfg(all(not(target_arch = "wasm32"), feature = "imaging_vello"))]
pub mod vello;
/// Masonry helpers for rendering retained scenes with `imaging_vello_cpu`.
#[cfg(feature = "imaging_vello_cpu")]
pub mod vello_cpu;
/// Masonry helpers for rendering retained scenes with `imaging_vello_hybrid`.
#[cfg(all(not(target_arch = "wasm32"), feature = "imaging_vello_hybrid"))]
pub mod vello_hybrid;

// The dependency only enables Vello CPU's worker pool for `imaging_vello_cpu`.
#[cfg(feature = "vello_cpu_multithreading")]
const _: Option<::vello_cpu::RenderSettings> = None;

pub use imaging::render::ImageRenderer;
#[cfg(not(target_arch = "wasm32"))]
pub use imaging_wgpu::TextureRenderer;

/// Backend-selected helpers for headless image rendering.
pub mod image_render {
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(feature = "imaging_vello"),
        feature = "imaging_skia"
    ))]
    pub use crate::skia::{BACKEND_NAME, Renderer, new_headless_renderer};
    #[cfg(all(not(target_arch = "wasm32"), feature = "imaging_vello"))]
    pub use crate::vello::{BACKEND_NAME, Renderer, new_headless_renderer};
    #[cfg(all(
        not(feature = "imaging_vello"),
        not(feature = "imaging_skia"),
        not(feature = "imaging_vello_hybrid"),
        feature = "imaging_vello_cpu"
    ))]
    pub use crate::vello_cpu::{BACKEND_NAME, Renderer, new_headless_renderer};
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(feature = "imaging_vello"),
        not(feature = "imaging_skia"),
        feature = "imaging_vello_hybrid"
    ))]
    pub use crate::vello_hybrid::{BACKEND_NAME, Renderer, new_headless_renderer};

    #[cfg(not(any(
        feature = "imaging_vello",
        feature = "imaging_vello_hybrid",
        feature = "imaging_vello_cpu",
        all(feature = "imaging_skia", not(target_arch = "wasm32"))
    )))]
    pub use self::no_backend::{BACKEND_NAME, Error, Renderer, new_headless_renderer};

    #[cfg(not(any(
        feature = "imaging_vello",
        feature = "imaging_vello_hybrid",
        feature = "imaging_vello_cpu",
        all(feature = "imaging_skia", not(target_arch = "wasm32"))
    )))]
    mod no_backend {
        use imaging::render::{
            ImageBufferFormat, ImageBufferTarget, ImageRendererError, RenderSource,
        };

        /// Error returned when no image-render backend feature is enabled.
        #[derive(Debug)]
        pub struct Error;

        impl core::fmt::Display for Error {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("no imaging backend feature selected")
            }
        }

        impl std::error::Error for Error {}

        /// Placeholder renderer used when no image-render backend feature is enabled.
        #[derive(Debug)]
        pub struct Renderer;

        /// Stable diagnostics name for the backend-less stub renderer.
        pub const BACKEND_NAME: &str = "no_backend";

        /// Create the backend-less stub renderer.
        pub fn new_headless_renderer() -> Result<Renderer, Error> {
            Err(Error)
        }

        impl imaging::render::ImageRenderer for Renderer {
            fn supported_image_formats(&self) -> Vec<ImageBufferFormat> {
                Vec::new()
            }

            fn render_source_into(
                &mut self,
                _: &mut dyn RenderSource,
                _: ImageBufferTarget<'_>,
            ) -> Result<(), ImageRendererError> {
                Err(ImageRendererError::backend(Error))
            }
        }
    }
}

/// A Masonry overlay layer ready to be composited into window space.
#[derive(Clone, Copy)]
pub struct Layer<'a> {
    /// The retained scene for this layer in layer-local coordinates.
    pub scene: &'a Scene,
    /// Transform from layer-local coordinates into window coordinates.
    pub transform: Affine,
}

impl core::fmt::Debug for Layer<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Layer")
            .field("scene", &"(Scene)")
            .field("transform", &self.transform)
            .finish()
    }
}

/// A flattened Masonry frame ready to be adapted to a concrete render target.
///
/// This is intentionally a single-target convenience type for Masonry's current rendering paths.
/// Future compositor-oriented work is expected to preserve more layer structure above this level.
#[derive(Clone, Copy, Debug)]
pub struct PreparedFrame<'a> {
    /// Frame width in physical pixels.
    pub width: u32,
    /// Frame height in physical pixels.
    pub height: u32,
    /// Window scale factor.
    pub scale_factor: f64,
    /// Background color to paint before replaying scene content.
    pub background_color: Color,
    /// Base retained scene in root coordinates.
    pub base: &'a Scene,
    /// Overlay layers in painter order.
    pub overlays: &'a [Layer<'a>],
}

impl<'a> PreparedFrame<'a> {
    /// Create a flattened Masonry frame from base content plus overlays.
    pub fn new(
        width: u32,
        height: u32,
        scale_factor: f64,
        background_color: Color,
        base: &'a Scene,
        overlays: &'a [Layer<'a>],
    ) -> Self {
        Self {
            width,
            height,
            scale_factor,
            background_color,
            base,
            overlays,
        }
    }
}

impl RenderSource for PreparedFrame<'_> {
    fn validate(&self) -> Result<(), ValidateError> {
        validate_layers(self.base, self.overlays)
    }

    fn paint_into(&mut self, sink: &mut dyn PaintSink) {
        {
            let mut painter = Painter::new(sink);
            painter.fill_rect(
                Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height)),
                self.background_color,
            );
        }

        let scale = Affine::scale(self.scale_factor);
        replay_transformed(self.base, sink, scale);
        for layer in self.overlays {
            replay_transformed(layer.scene, sink, scale * layer.transform);
        }
    }
}

/// A rectangle of a frame in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PixelRect {
    /// Create a pixel rectangle.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge, exclusive.
    pub const fn right(&self) -> u32 {
        self.x + self.width
    }

    /// Bottom edge, exclusive.
    pub const fn bottom(&self) -> u32 {
        self.y + self.height
    }

    /// Number of pixels covered.
    pub const fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Whether the rectangle covers no pixels.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    /// Whether the two rectangles share at least one pixel.
    pub fn intersects(&self, other: &Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether `other` lies entirely inside this rectangle.
    pub fn contains(&self, other: &Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

/// Convert window-space damage in logical pixels to merged physical pixel rectangles.
///
/// Each rectangle is scaled, rounded outwards, padded by `margin` pixels for antialiasing, snapped
/// outwards to multiples of `align` (except at the frame's right and bottom edges) and clamped to
/// `width` × `height`. Overlapping rectangles, and rectangles whose union wastes no area, are
/// merged, as are the cheapest pairs until at most `max_rects` remain.
pub fn damage_to_pixel_rects(
    damage: &[Rect],
    scale_factor: f64,
    (width, height): (u32, u32),
    margin: u32,
    (align_x, align_y): (u32, u32),
    max_rects: usize,
) -> Vec<PixelRect> {
    let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let margin = f64::from(margin);
    let mut rects: Vec<PixelRect> = Vec::with_capacity(damage.len());
    for rect in damage {
        let scaled = Affine::scale(scale_factor)
            .transform_rect_bbox(*rect)
            .inflate(margin, margin);
        // A non-finite rectangle damages the whole frame.
        let scaled = if scaled.is_finite() { scaled } else { bounds };
        let clamped = scaled.intersect(bounds).expand();
        if !(clamped.width() > 0.0 && clamped.height() > 0.0) {
            continue;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to the frame, whose size fits in u32"
        )]
        let to_u32 = |v: f64| v as u32;
        let snap_down = |v: u32, align: u32| v - v % align.max(1);
        let snap_up = |v: u32, align: u32, limit: u32| v.next_multiple_of(align.max(1)).min(limit);
        let x0 = snap_down(to_u32(clamped.x0), align_x);
        let y0 = snap_down(to_u32(clamped.y0), align_y);
        let x1 = snap_up(to_u32(clamped.x1), align_x, width);
        let y1 = snap_up(to_u32(clamped.y1), align_y, height);
        rects.push(PixelRect::new(x0, y0, x1 - x0, y1 - y0));
    }
    merge_pixel_rects(&mut rects, max_rects.max(1));
    rects
}

/// Merge rectangles that overlap, or whose union wastes little area, until at most `max_rects`
/// remain.
fn merge_pixel_rects(rects: &mut Vec<PixelRect>, max_rects: usize) {
    loop {
        let mut best: Option<(usize, usize, i64)> = None;
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (a, b) = (rects[i], rects[j]);
                let union = a.union(&b);
                #[expect(clippy::cast_possible_wrap, reason = "pixel areas fit in i64")]
                let waste = union.area() as i64 - a.area() as i64 - b.area() as i64;
                let cheap = a.intersects(&b) || waste <= 0;
                if (cheap || rects.len() > max_rects) && best.is_none_or(|(_, _, w)| waste < w) {
                    best = Some((i, j, waste));
                }
            }
        }
        let Some((i, j, _)) = best else {
            return;
        };
        let merged = rects[i].union(&rects[j]);
        rects[i] = merged;
        rects.swap_remove(j);
    }
}

/// One rectangle of a [`PreparedFrame`].
///
/// Rendering a region at the frame's full size produces the frame's pixels inside `rect` and
/// transparent pixels elsewhere. Draws whose conservative bounds miss the rectangle are skipped,
/// and the rest is clipped to it rather than translated: brushes such as images and gradients are
/// evaluated in single precision, so a translated render would sample them at slightly different
/// positions than a full one.
#[derive(Clone, Copy, Debug)]
pub struct FrameRegion<'a> {
    /// The full frame.
    pub frame: PreparedFrame<'a>,
    /// The rectangle of the frame to render.
    pub rect: PixelRect,
}

impl RenderSource for FrameRegion<'_> {
    fn validate(&self) -> Result<(), ValidateError> {
        validate_layers(self.frame.base, self.frame.overlays)
    }

    fn paint_into(&mut self, sink: &mut dyn PaintSink) {
        let rect = Rect::new(
            f64::from(self.rect.x),
            f64::from(self.rect.y),
            f64::from(self.rect.right()),
            f64::from(self.rect.bottom()),
        );
        let mut painter = Painter::new(sink);
        painter.push_fill_clip(rect);
        painter.fill_rect(rect, self.frame.background_color);
        {
            // Draws outside the region would still be flattened and tiled before the clip
            // discards them, so they are skipped here.
            let mut culled = cull::CullingSink::new(painter.sink_mut(), rect.inflate(1.0, 1.0));
            let scale = Affine::scale(self.frame.scale_factor);
            replay_transformed(self.frame.base, &mut culled, scale);
            for layer in self.frame.overlays {
                replay_transformed(layer.scene, &mut culled, scale * layer.transform);
            }
        }
        painter.pop_clip();
    }
}

fn validate_layers(base: &Scene, overlays: &[Layer<'_>]) -> Result<(), ValidateError> {
    base.validate()?;
    for layer in overlays {
        layer.scene.validate()?;
    }
    Ok(())
}
