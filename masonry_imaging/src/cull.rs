// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Dropping draws that cannot touch a region before they reach a renderer.

use imaging::record::Glyph;
use imaging::{
    BlurredRoundedRect, ClipRef, Composite, FillRef, GeometryRef, GlyphRunRef, GroupRef, PaintSink,
    StrokeRef,
};
use kurbo::{Join, Rect, Shape as _, Stroke};
use peniko::{Compose, Style};

/// Forwards commands to `inner`, skipping draws whose conservative bounds miss `viewport`.
///
/// Draws inside groups whose result can spread beyond their content (filters, or blend modes
/// other than source-over) are always forwarded, as are draws with such blend modes. Clips and
/// groups are always forwarded, so the command structure is unchanged.
pub(crate) struct CullingSink<'a> {
    inner: &'a mut dyn PaintSink,
    viewport: Rect,
    /// For each open group, whether it spreads.
    groups: Vec<bool>,
    spreading: usize,
    glyphs: Vec<Glyph>,
}

impl<'a> CullingSink<'a> {
    pub(crate) fn new(inner: &'a mut dyn PaintSink, viewport: Rect) -> Self {
        Self {
            inner,
            viewport,
            groups: Vec::new(),
            spreading: 0,
            glyphs: Vec::new(),
        }
    }

    fn keep(&self, composite: Composite, bounds: impl FnOnce() -> Option<Rect>) -> bool {
        if self.spreading > 0 || composite.blend.compose != Compose::SrcOver {
            return true;
        }
        bounds().is_none_or(|bounds| {
            !bounds.is_finite()
                || (bounds.x0 < self.viewport.x1
                    && bounds.x1 > self.viewport.x0
                    && bounds.y0 < self.viewport.y1
                    && bounds.y1 > self.viewport.y0)
        })
    }
}

fn geometry_bounds(geometry: &GeometryRef<'_>) -> Rect {
    match geometry {
        GeometryRef::Rect(rect) => *rect,
        GeometryRef::RoundedRect(rect) => rect.rect(),
        GeometryRef::Path(path) => path.bounding_box(),
        GeometryRef::OwnedPath(path) => path.bounding_box(),
    }
}

/// How far a stroke can extend beyond its path.
fn stroke_pad(stroke: &Stroke) -> f64 {
    let join = if stroke.join == Join::Miter {
        stroke.miter_limit.max(1.0)
    } else {
        1.0
    };
    (stroke.width.abs() * 0.5 * join.max(std::f64::consts::SQRT_2)).max(1.0)
}

impl PaintSink for CullingSink<'_> {
    fn push_clip(&mut self, clip: ClipRef<'_>) {
        self.inner.push_clip(clip);
    }

    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }

    fn push_group(&mut self, group: GroupRef<'_>) {
        let spreads =
            !group.filters.is_empty() || group.composite.blend.compose != Compose::SrcOver;
        self.groups.push(spreads);
        self.spreading += usize::from(spreads);
        self.inner.push_group(group);
    }

    fn pop_group(&mut self) {
        if self.groups.pop() == Some(true) {
            self.spreading -= 1;
        }
        self.inner.pop_group();
    }

    fn fill(&mut self, draw: FillRef<'_>) {
        if self.keep(draw.composite, || {
            Some(
                draw.transform
                    .transform_rect_bbox(geometry_bounds(&draw.shape)),
            )
        }) {
            self.inner.fill(draw);
        }
    }

    fn stroke(&mut self, draw: StrokeRef<'_>) {
        if self.keep(draw.composite, || {
            let pad = stroke_pad(draw.stroke);
            Some(
                draw.transform
                    .transform_rect_bbox(geometry_bounds(&draw.shape).inflate(pad, pad)),
            )
        }) {
            self.inner.stroke(draw);
        }
    }

    fn glyph_run(&mut self, draw: GlyphRunRef<'_>, glyphs: &mut dyn Iterator<Item = Glyph>) {
        let mut buffer = std::mem::take(&mut self.glyphs);
        buffer.clear();
        buffer.extend(glyphs);
        // Each glyph is assumed to fit two ems above its origin, one below, one to the left and
        // three to the right, as in Masonry's damage tracking.
        let keep = self.keep(draw.composite, || {
            let mut origins: Option<Rect> = None;
            for glyph in &buffer {
                let point = Rect::new(
                    f64::from(glyph.x),
                    f64::from(glyph.y),
                    f64::from(glyph.x),
                    f64::from(glyph.y),
                );
                origins = Some(origins.map_or(point, |rect| rect.union(point)));
            }
            let origins = origins?;
            let em = f64::from(draw.font_size).abs().max(1.0);
            let mut glyph_box = Rect::new(-em, -2.0 * em, 3.0 * em, em);
            if let Some(transform) = draw.glyph_transform {
                glyph_box = transform.transform_rect_bbox(glyph_box);
            }
            let pad = draw.font_embolden.x.abs().max(draw.font_embolden.y.abs())
                + match draw.style {
                    Style::Fill(_) => 0.0,
                    Style::Stroke(stroke) => stroke_pad(stroke),
                };
            Some(draw.transform.transform_rect_bbox(Rect::new(
                origins.x0 + glyph_box.x0 - pad,
                origins.y0 + glyph_box.y0 - pad,
                origins.x1 + glyph_box.x1 + pad,
                origins.y1 + glyph_box.y1 + pad,
            )))
        });
        if keep {
            self.inner.glyph_run(draw, &mut buffer.iter().copied());
        }
        self.glyphs = buffer;
    }

    fn blurred_rounded_rect(&mut self, draw: BlurredRoundedRect) {
        if self.keep(draw.composite, || {
            let pad = 3.0 * draw.std_dev.abs() + 1.0;
            Some(
                draw.transform
                    .transform_rect_bbox(draw.rect.inflate(pad, pad)),
            )
        }) {
            self.inner.blurred_rounded_rect(draw);
        }
    }
}
