// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Window damage: which parts of the window changed between two painted frames.
//!
//! The paint pass records, for every widget, where its retained scenes land in the window. Each
//! frame it compares that record with the new one and damages:
//!
//! - the old and new rectangles of widgets that moved, resized, changed clip, were stashed or
//!   unstashed, or switched to or from an external layer;
//! - the rectangles of draws that changed between a repainted widget's old and new scenes;
//! - the old rectangles of removed widgets, reported as they are removed, and the new rectangles
//!   of added ones;
//! - the old and new subtree rectangles of widgets whose children were reordered.
//!
//! Rectangles are conservative bounds of what was drawn, intersected with the window-space bounds
//! of ancestor clips. They cover what widgets actually paint, including paint outside their
//! layout boxes, so widgets do not need accurate paint insets for damage to be correct.

use kurbo::{Affine, Join, Rect, Shape, Stroke};
use peniko::{Compose, Style};

use crate::core::WidgetId;
use crate::dpi::PhysicalSize;
use crate::imaging::Composite;
use crate::imaging::record::{Clip, Command, Draw, Geometry, GlyphRun, Group, Scene};

/// The region of the window that changed since the previous frame.
///
/// Returned with every [`VisualLayerPlan`](crate::app::VisualLayerPlan). A host that keeps the
/// previous frame's pixels only needs to re-rasterise and present this region. Pixels outside it
/// are identical to the previous frame, as long as the host also repaints everything after
/// discarding or failing to present a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Damage {
    /// The whole window must be repainted.
    ///
    /// Reported on the first frame, after resizes, scale factor and default property changes,
    /// while debug painting or the inspector overlay is active, and when the damaged region covers
    /// most of the window.
    Full,
    /// Only these window-space rectangles, in logical pixels, changed.
    ///
    /// The rectangles may overlap. The list is empty when nothing visible changed.
    Region(Vec<Rect>),
}

impl Damage {
    /// Whether the whole window must be repainted.
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether nothing visible changed.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Region(rects) if rects.is_empty())
    }

    /// The damaged rectangles, or `None` for full damage.
    pub fn rects(&self) -> Option<&[Rect]> {
        match self {
            Self::Full => None,
            Self::Region(rects) => Some(rects),
        }
    }

    /// The smallest rectangle containing all damage, clipped to `window`.
    ///
    /// Returns `None` when nothing changed.
    pub fn bounds(&self, window: Rect) -> Option<Rect> {
        match self {
            Self::Full => Some(window),
            Self::Region(rects) => rects
                .iter()
                .filter_map(|rect| clip_to(*rect, window))
                .reduce(|a, b| a.union(b)),
        }
    }
}

/// Bounds used for content whose extent cannot be determined, such as filtered groups.
///
/// It is finite so that it survives affine transforms, and is always clipped to the window.
const UNBOUNDED: Rect = Rect::new(-1.0e9, -1.0e9, 1.0e9, 1.0e9);

/// Damage covering more than this fraction of the window is reported as [`Damage::Full`].
const FULL_DAMAGE_FRACTION: f64 = 0.6;

/// At most this many damage rectangles are reported; more are merged.
const MAX_DAMAGE_RECTS: usize = 16;

/// How far the scene diff looks ahead to resynchronise after an inserted or removed draw.
const RESYNC_WINDOW: usize = 16;

/// Intersects two rectangles, returning `None` if the result has no area.
pub(crate) fn clip_to(rect: Rect, clip: Rect) -> Option<Rect> {
    let rect = rect.intersect(clip);
    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

// --- MARK: TRACKER

/// The paint slots of a widget, in the order of `scene_cache` entries.
pub(crate) const SLOTS: usize = 3;
/// Index of the slot clipped by the widget's own clip path.
pub(crate) const PAINT_SLOT: usize = 1;

/// What the paint pass recorded about a widget in the previous frame.
///
/// Stored in the widget's state, so a removed widget takes its record with it; removal reports
/// the record's rectangle through [`DamageTracker::removed`].
#[derive(Clone, Debug)]
pub(crate) struct PaintRecord {
    /// Content-box to window transform.
    transform: Affine,
    /// Window-space bounds of ancestor clips; clips pre-paint and post-paint.
    outer_clip: Rect,
    /// `outer_clip` intersected with the widget's own clip; clips paint and children.
    inner_clip: Rect,
    /// Whether the widget's scenes were painted (it was not stashed or external).
    painted: bool,
    /// Whether the widget was realized as an external layer.
    external: bool,
    /// Content-box bounds of each slot's scene, `None` when empty.
    bounds: [Option<Rect>; SLOTS],
    /// Window-space rectangle covered by this widget's own content.
    rect: Option<Rect>,
    /// Window-space rectangle covered by this widget and its descendants.
    subtree: Option<Rect>,
    /// Hash of the child id sequence, `None` until the children are first recorded.
    children_hash: Option<u64>,
    /// Child ids in paint order.
    children: Vec<WidgetId>,
}

/// The paint pass's view of a widget in the current frame.
pub(crate) struct PaintGeometry {
    /// Content-box to window transform.
    pub(crate) transform: Affine,
    /// Window-space ancestor clip bounds.
    pub(crate) outer_clip: Rect,
    /// `outer_clip` intersected with the widget's own clip bounds.
    pub(crate) inner_clip: Rect,
    /// Whether the widget's scenes are painted.
    pub(crate) painted: bool,
    /// Whether the widget is realized as an external layer, with these window-space bounds.
    pub(crate) external: Option<Rect>,
}

impl PaintGeometry {
    fn slot_clip(&self, slot: usize) -> Rect {
        if slot == PAINT_SLOT {
            self.inner_clip
        } else {
            self.outer_clip
        }
    }

    /// Window-space rectangle of content-box `local` bounds painted in `slot`.
    fn window_rect(&self, slot: usize, local: Option<Rect>) -> Option<Rect> {
        clip_to(
            self.transform.transform_rect_bbox(local?),
            self.slot_clip(slot),
        )
    }
}

/// How a repainted slot's scene changed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlotChange {
    /// Content-box bounds of the new scene.
    bounds: Option<Rect>,
    /// Content-box bounds of the draws that differ, `None` if the scenes are equivalent.
    changed: Option<Rect>,
}

/// Damage bookkeeping carried between paint passes.
#[derive(Default)]
pub(crate) struct DamageTracker {
    /// Spare scene that receives a slot's previous contents while it repaints.
    pub(crate) scratch: Scene,
    window: Rect,
    full: bool,
    rects: Vec<Rect>,
    /// Rectangles of widgets removed since the previous frame.
    removed: Vec<Rect>,
    /// Size and scale factor of the previous frame, `None` before the first frame.
    last_frame: Option<(PhysicalSize<u32>, f64)>,
    last_layers: Vec<WidgetId>,
    /// Whether the previous frame was forced to full damage by overlays outside widget scenes.
    last_always_full: bool,
    /// Set when something outside the widget tree changes every widget's appearance.
    pub(crate) force_full: bool,
}

impl std::fmt::Debug for DamageTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DamageTracker")
            .field("full", &self.full)
            .field("rects", &self.rects)
            .finish_non_exhaustive()
    }
}

impl DamageTracker {
    /// Starts a frame for a window of the given physical size, scale factor and logical bounds.
    ///
    /// `always_full` forces full damage for this frame and the next, for overlays that are
    /// painted outside widget scenes and must be erased once they are gone.
    pub(crate) fn begin_frame(
        &mut self,
        size: PhysicalSize<u32>,
        scale_factor: f64,
        window: Rect,
        layers: &[WidgetId],
        always_full: bool,
    ) {
        self.window = window;
        self.rects.clear();
        self.full = std::mem::take(&mut self.force_full)
            || always_full
            || std::mem::replace(&mut self.last_always_full, always_full)
            || self.last_frame != Some((size, scale_factor))
            || reordered(&self.last_layers, layers);
        self.last_frame = Some((size, scale_factor));
        self.last_layers.clear();
        self.last_layers.extend_from_slice(layers);
        for rect in std::mem::take(&mut self.removed) {
            self.add(Some(rect));
        }
    }

    /// Damages the rectangle of a widget removed from the tree.
    pub(crate) fn removed(&mut self, record: Option<&PaintRecord>) {
        if let Some(rect) = record.and_then(|record| record.rect) {
            self.removed.push(rect);
        }
    }

    /// Damages a window-space rectangle.
    pub(crate) fn add(&mut self, rect: Option<Rect>) {
        if self.full {
            return;
        }
        if let Some(rect) = rect.and_then(|rect| clip_to(rect, self.window)) {
            push_rect(&mut self.rects, rect, MAX_DAMAGE_RECTS);
        }
    }

    /// Diffs a repainted slot's previous scene, now in `scratch`, against its new contents.
    pub(crate) fn diff_slot(&self, new: &Scene) -> SlotChange {
        let tokens = tokenize(new);
        let bounds = tokens.iter().map(|token| token.effect).fold(None, union);
        let changed = if self.scratch == *new {
            None
        } else {
            diff_tokens(&self.scratch, &tokenize(&self.scratch), new, &tokens)
        };
        SlotChange { bounds, changed }
    }

    /// Records a painted widget and damages whatever changed since its previous record.
    ///
    /// `changes` holds the slots that repainted this frame; `cached` provides scenes for slots
    /// whose bounds are not yet known. Returns the window-space rectangle of the widget's own
    /// content.
    pub(crate) fn update_widget(
        &mut self,
        record: &mut Option<PaintRecord>,
        geometry: &PaintGeometry,
        changes: [Option<SlotChange>; SLOTS],
        cached: impl Fn(usize) -> Option<Rect>,
    ) -> Option<Rect> {
        let previous = record.as_ref();
        let mut bounds = [None; SLOTS];
        for (slot, bound) in bounds.iter_mut().enumerate() {
            *bound = match (changes[slot], previous) {
                (Some(change), _) => change.bounds,
                (None, Some(previous)) => previous.bounds[slot],
                (None, None) => cached(slot),
            };
        }

        let rect = if let Some(external) = geometry.external {
            clip_to(external, geometry.outer_clip)
        } else if geometry.painted {
            (0..SLOTS)
                .map(|slot| geometry.window_rect(slot, bounds[slot]))
                .fold(None, union)
        } else {
            None
        };

        let mut damage: [Option<Rect>; SLOTS + 1] = [None; SLOTS + 1];
        match previous {
            None => damage[0] = rect,
            Some(previous)
                if previous.transform != geometry.transform
                    || previous.outer_clip != geometry.outer_clip
                    || previous.inner_clip != geometry.inner_clip
                    || previous.painted != geometry.painted
                    || previous.external != geometry.external.is_some() =>
            {
                damage[0] = previous.rect;
                damage[1] = rect;
            }
            Some(_) if geometry.painted && geometry.external.is_none() => {
                for (slot, change) in changes.iter().enumerate() {
                    if let Some(change) = change {
                        damage[slot + 1] = geometry.window_rect(slot, change.changed);
                    }
                }
            }
            Some(_) => {}
        }
        for rect in damage {
            self.add(rect);
        }

        let record = record.get_or_insert_with(|| PaintRecord {
            transform: geometry.transform,
            outer_clip: geometry.outer_clip,
            inner_clip: geometry.inner_clip,
            painted: false,
            external: false,
            bounds: [None; SLOTS],
            rect: None,
            subtree: None,
            children_hash: None,
            children: Vec::new(),
        });
        record.transform = geometry.transform;
        record.outer_clip = geometry.outer_clip;
        record.inner_clip = geometry.inner_clip;
        record.painted = geometry.painted;
        record.external = geometry.external.is_some();
        record.bounds = bounds;
        record.rect = rect;
        rect
    }

    /// Records a widget's subtree rectangle and child order after its children painted.
    ///
    /// `children` is only called when the child sequence hash changed.
    pub(crate) fn update_subtree(
        &mut self,
        record: &mut Option<PaintRecord>,
        subtree: Option<Rect>,
        children_hash: u64,
        children: impl FnOnce() -> Vec<WidgetId>,
    ) {
        let Some(record) = record else {
            debug_panic!("update_subtree called before update_widget");
            return;
        };
        let old_subtree = std::mem::replace(&mut record.subtree, subtree);
        if record.children_hash == Some(children_hash) {
            return;
        }
        let new_children = children();
        let was_reordered = reordered(&record.children, &new_children);
        record.children_hash = Some(children_hash);
        record.children = new_children;
        if was_reordered {
            self.add(old_subtree);
            self.add(subtree);
        }
    }

    /// Ends the frame and returns its damage.
    pub(crate) fn finish_frame(&mut self) -> Damage {
        if self.full {
            return Damage::Full;
        }
        let rects = std::mem::take(&mut self.rects);
        let damaged: f64 = rects.iter().map(Rect::area).sum();
        if damaged > self.window.area() * FULL_DAMAGE_FRACTION {
            return Damage::Full;
        }
        Damage::Region(rects)
    }
}

/// Whether the ids present in both sequences appear in a different relative order.
fn reordered(old: &[WidgetId], new: &[WidgetId]) -> bool {
    if old == new {
        return false;
    }
    let common_old = old.iter().filter(|id| new.contains(id));
    let common_new = new.iter().filter(|id| old.contains(id));
    !common_old.eq(common_new)
}

/// Adds a rectangle to a list of non-overlapping rectangles.
///
/// Rectangles it overlaps are absorbed into it. When the list is full, it is merged with the
/// rectangle whose union with it grows least. Each call is linear in `max`.
fn push_rect(rects: &mut Vec<Rect>, mut rect: Rect, max: usize) {
    loop {
        while let Some(i) = rects
            .iter()
            .position(|other| clip_to(*other, rect).is_some())
        {
            rect = rect.union(rects.swap_remove(i));
        }
        if rects.len() < max {
            rects.push(rect);
            return;
        }
        let growth = |other: &Rect| other.union(rect).area() - other.area();
        let Some(i) =
            (0..rects.len()).min_by(|&a, &b| growth(&rects[a]).total_cmp(&growth(&rects[b])))
        else {
            rects.push(rect);
            return;
        };
        // The union can overlap others, so absorb again.
        rect = rect.union(rects.swap_remove(i));
    }
}

// --- MARK: SCENE BOUNDS

fn geometry_bounds(geometry: &Geometry) -> Rect {
    match geometry {
        Geometry::Rect(rect) => *rect,
        Geometry::RoundedRect(rect) => rect.rect(),
        Geometry::Path(path) => path.bounding_box(),
    }
}

/// How far a stroke can extend beyond its path.
fn stroke_pad(stroke: &Stroke) -> f64 {
    let join = if stroke.join == Join::Miter {
        stroke.miter_limit.max(1.0)
    } else {
        1.0
    };
    // Square caps reach half the width diagonally; hairlines still cover a pixel.
    (stroke.width.abs() * 0.5 * join.max(std::f64::consts::SQRT_2)).max(1.0)
}

/// Whether a draw or group only affects the pixels it covers.
fn is_source_over(composite: Composite) -> bool {
    composite.blend.compose == Compose::SrcOver
}

/// Conservative bounds of a glyph run.
///
/// Glyph outlines are not available here, so each glyph is assumed to fit in a box reaching two
/// ems above its origin, one below, one to the left and three to the right, which covers wide
/// ligatures and stacked diacritics in common fonts.
fn glyph_run_bounds(run: &GlyphRun) -> Option<Rect> {
    let mut origins: Option<Rect> = None;
    for glyph in &run.glyphs {
        let point = Rect::new(
            f64::from(glyph.x),
            f64::from(glyph.y),
            f64::from(glyph.x),
            f64::from(glyph.y),
        );
        origins = Some(origins.map_or(point, |rect| rect.union(point)));
    }
    let origins = origins?;
    let em = f64::from(run.font_size).abs().max(1.0);
    let mut glyph_box = Rect::new(-em, -2.0 * em, 3.0 * em, em);
    if let Some(transform) = run.glyph_transform {
        glyph_box = transform.transform_rect_bbox(glyph_box);
    }
    let embolden = run.font_embolden.x.abs().max(run.font_embolden.y.abs());
    let stroke = match &run.style {
        Style::Fill(_) => 0.0,
        Style::Stroke(stroke) => stroke_pad(stroke),
    };
    let pad = embolden + stroke;
    let local = Rect::new(
        origins.x0 + glyph_box.x0 - pad,
        origins.y0 + glyph_box.y0 - pad,
        origins.x1 + glyph_box.x1 + pad,
        origins.y1 + glyph_box.y1 + pad,
    );
    Some(run.transform.transform_rect_bbox(local))
}

/// Conservative bounds of one draw, ignoring clips.
fn draw_bounds(draw: &Draw) -> Option<Rect> {
    match draw {
        Draw::Fill {
            transform,
            shape,
            composite,
            ..
        } => Some(if is_source_over(*composite) {
            transform.transform_rect_bbox(geometry_bounds(shape))
        } else {
            UNBOUNDED
        }),
        Draw::Stroke {
            transform,
            stroke,
            shape,
            composite,
            ..
        } => Some(if is_source_over(*composite) {
            let pad = stroke_pad(stroke);
            transform.transform_rect_bbox(geometry_bounds(shape).inflate(pad, pad))
        } else {
            UNBOUNDED
        }),
        Draw::GlyphRun(run) => {
            if is_source_over(run.composite) {
                glyph_run_bounds(run)
            } else {
                Some(UNBOUNDED)
            }
        }
        Draw::BlurredRoundedRect(draw) => Some(if is_source_over(draw.composite) {
            // The gaussian kernel is negligible beyond three standard deviations.
            let pad = 3.0 * draw.std_dev.abs() + 1.0;
            draw.transform
                .transform_rect_bbox(draw.rect.inflate(pad, pad))
        } else {
            UNBOUNDED
        }),
    }
}

fn clip_bounds(clip: &Clip) -> Rect {
    match clip {
        Clip::Fill {
            transform, shape, ..
        } => transform.transform_rect_bbox(geometry_bounds(shape)),
        Clip::Stroke {
            transform,
            shape,
            stroke,
        } => {
            let pad = stroke_pad(stroke);
            transform.transform_rect_bbox(geometry_bounds(shape).inflate(pad, pad))
        }
    }
}

/// Whether a group's result can reach beyond the draws inside it.
fn group_spreads(group: &Group) -> bool {
    !group.filters.is_empty() || !is_source_over(group.composite)
}

/// A scene command the diff compares, with its effect on the window.
#[derive(Clone, Copy)]
struct Token<'a> {
    command: &'a Command,
    /// Scene-space bounds of the pixels this command can change, within its clips.
    effect: Option<Rect>,
}

impl Token<'_> {
    fn is_draw(&self) -> bool {
        matches!(self.command, Command::Draw(_))
    }
}

/// Reduces a scene to tokens, computing each command's clipped effect.
///
/// Context annotations are dropped: they do not affect pixels.
fn tokenize(scene: &Scene) -> Vec<Token<'_>> {
    let mut tokens = Vec::with_capacity(scene.commands().len());
    // The clip bounds, and whether a group whose result spreads beyond its draws is open.
    let mut stack: Vec<(Rect, bool)> = Vec::new();
    let mut current = (UNBOUNDED, false);
    for command in scene.commands() {
        let effect = match command {
            Command::PushContext(_) | Command::PopContext => continue,
            Command::PushClip(id) => {
                stack.push(current);
                current.0 = current.0.intersect(clip_bounds(scene.clip(*id)));
                None
            }
            Command::PopClip | Command::PopGroup => {
                current = stack.pop().unwrap_or((UNBOUNDED, false));
                None
            }
            Command::PushGroup(id) => {
                stack.push(current);
                let group = scene.group(*id);
                if let Some(clip) = &group.clip {
                    current.0 = current.0.intersect(clip_bounds(clip));
                }
                let spreads = group_spreads(group);
                current.1 |= spreads;
                // A spreading group changes its whole clip region, even with nothing inside.
                spreads.then(|| clip_to(UNBOUNDED, current.0)).flatten()
            }
            Command::Draw(id) => {
                let bounds = if current.1 {
                    Some(UNBOUNDED)
                } else {
                    draw_bounds(scene.draw_op(*id))
                };
                bounds.and_then(|bounds| clip_to(bounds, current.0))
            }
        };
        tokens.push(Token { command, effect });
    }
    tokens
}

/// Whether two tokens from different scenes have identical effects in identical contexts.
fn same(old: &Scene, a: &Token<'_>, new: &Scene, b: &Token<'_>) -> bool {
    match (a.command, b.command) {
        (Command::Draw(a), Command::Draw(b)) => old.draw_op(*a) == new.draw_op(*b),
        (Command::PushClip(a), Command::PushClip(b)) => old.clip(*a) == new.clip(*b),
        (Command::PushGroup(a), Command::PushGroup(b)) => {
            let (a, b) = (old.group(*a), new.group(*b));
            a.clip == b.clip
                && a.filters == b.filters
                && a.composite == b.composite
                && match (&a.mask, &b.mask) {
                    (None, None) => true,
                    (Some(a), Some(b)) => {
                        a.transform == b.transform && old.mask(a.mask) == new.mask(b.mask)
                    }
                    _ => false,
                }
        }
        (Command::PopClip, Command::PopClip) | (Command::PopGroup, Command::PopGroup) => true,
        _ => false,
    }
}

/// Conservative content bounds of a scene, `None` if it draws nothing.
pub(crate) fn scene_bounds(scene: &Scene) -> Option<Rect> {
    tokenize(scene)
        .iter()
        .map(|token| token.effect)
        .fold(None, union)
}

/// Bounds of the pixels that can differ between renderings of two tokenized scenes, `None` if
/// none can.
///
/// The common prefix and suffix are skipped. If the remaining commands have the same clip and
/// group structure, draws are matched in order within each structural segment, and only unmatched
/// draws are damaged; otherwise every remaining command is.
fn diff_tokens(old: &Scene, a: &[Token<'_>], new: &Scene, b: &[Token<'_>]) -> Option<Rect> {
    let eq = |i: usize, j: usize| same(old, &a[i], new, &b[j]);

    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && eq(prefix, prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && eq(a.len() - 1 - suffix, b.len() - 1 - suffix)
    {
        suffix += 1;
    }
    let mid_a = prefix..a.len() - suffix;
    let mid_b = prefix..b.len() - suffix;

    let structure_a: Vec<usize> = mid_a.clone().filter(|&i| !a[i].is_draw()).collect();
    let structure_b: Vec<usize> = mid_b.clone().filter(|&j| !b[j].is_draw()).collect();
    let same_structure = structure_a.len() == structure_b.len()
        && structure_a
            .iter()
            .zip(&structure_b)
            .all(|(&i, &j)| eq(i, j));

    let mut damage = None;
    if !same_structure {
        for token in a[mid_a].iter().chain(&b[mid_b]) {
            damage = union(damage, token.effect);
        }
        return damage;
    }

    // Diff the draws between consecutive structural commands.
    let mut start_a = mid_a.start;
    let mut start_b = mid_b.start;
    let ends_a = structure_a.iter().copied().chain([mid_a.end]);
    let ends_b = structure_b.iter().copied().chain([mid_b.end]);
    for (end_a, end_b) in ends_a.zip(ends_b) {
        let (mut i, mut j) = (start_a, start_b);
        while i < end_a && j < end_b {
            if eq(i, j) {
                i += 1;
                j += 1;
                continue;
            }
            // Resynchronise on the nearest draw that was inserted or removed.
            let skip = (1..=RESYNC_WINDOW).find_map(|k| {
                if i + k < end_a && eq(i + k, j) {
                    Some((k, 0))
                } else if j + k < end_b && eq(i, j + k) {
                    Some((0, k))
                } else {
                    None
                }
            });
            let (skip_a, skip_b) = skip.unwrap_or((1, 1));
            for token in &a[i..i + skip_a] {
                damage = union(damage, token.effect);
            }
            for token in &b[j..j + skip_b] {
                damage = union(damage, token.effect);
            }
            i += skip_a;
            j += skip_b;
        }
        for token in a[i..end_a].iter().chain(&b[j..end_b]) {
            damage = union(damage, token.effect);
        }
        start_a = end_a + 1;
        start_b = end_b + 1;
    }
    damage
}

#[cfg(test)]
mod tests {
    use kurbo::{Affine, Rect, Stroke};
    use peniko::{BlendMode, Color, Compose, Mix};

    use super::{Damage, diff_tokens, push_rect, scene_bounds, tokenize};
    use crate::imaging::record::Scene;
    use crate::imaging::{Composite, Filter, GroupRef, Painter};

    const RED: Color = Color::from_rgb8(255, 0, 0);
    const BLUE: Color = Color::from_rgb8(0, 0, 255);

    fn diff(old: &Scene, new: &Scene) -> Option<Rect> {
        if old == new {
            return None;
        }
        diff_tokens(old, &tokenize(old), new, &tokenize(new))
    }

    /// Ten stacked lines, each a filled rectangle; `caret` adds a caret before them.
    fn lines(changed: Option<usize>, caret: Option<f64>) -> Scene {
        let mut scene = Scene::new();
        let mut painter = Painter::new(&mut scene);
        if let Some(x) = caret {
            painter.fill_rect(Rect::new(x, 0., x + 1., 10.), BLUE);
        }
        for line in 0..10 {
            let y = f64::from(line) * 20.;
            let width = if changed == Some(line as usize) {
                150.
            } else {
                100.
            };
            painter.fill_rect(Rect::new(0., y, width, y + 10.), RED);
        }
        scene
    }

    #[test]
    fn identical_scenes_have_no_damage() {
        assert_eq!(diff(&lines(None, Some(5.)), &lines(None, Some(5.))), None);
    }

    #[test]
    fn a_changed_draw_damages_only_itself() {
        let damage = diff(&lines(None, None), &lines(Some(4), None)).unwrap();
        assert_eq!(damage, Rect::new(0., 80., 150., 90.));
    }

    #[test]
    fn an_inserted_draw_damages_only_itself() {
        let damage = diff(&lines(None, None), &lines(None, Some(30.))).unwrap();
        assert_eq!(damage, Rect::new(30., 0., 31., 10.));
    }

    #[test]
    fn a_moved_caret_and_an_edited_line_damage_both() {
        let damage = diff(&lines(None, Some(5.)), &lines(Some(7), Some(40.))).unwrap();
        // The caret's old and new rectangles, and line 7; lines between them are untouched by
        // pixels but inside the bounding rectangle.
        assert_eq!(damage, Rect::new(0., 0., 150., 150.));
        let caret_only = diff(&lines(None, Some(5.)), &lines(None, Some(40.))).unwrap();
        assert_eq!(caret_only, Rect::new(5., 0., 41., 10.));
    }

    #[test]
    fn structural_changes_damage_the_changed_range() {
        let old = lines(None, None);
        let mut new = Scene::new();
        {
            let mut painter = Painter::new(&mut new);
            painter.push_fill_clip(Rect::new(0., 0., 50., 400.));
            for line in 0..10 {
                let y = f64::from(line) * 20.;
                painter.fill_rect(Rect::new(0., y, 100., y + 10.), RED);
            }
            painter.pop_clip();
        }
        // Every line is now clipped: the whole content changes.
        assert_eq!(diff(&old, &new).unwrap(), Rect::new(0., 0., 100., 190.));
    }

    #[test]
    fn spreading_groups_damage_their_clip() {
        let make = |color: Color| {
            let mut scene = Scene::new();
            let mut painter = Painter::new(&mut scene);
            painter.push_fill_clip(Rect::new(0., 0., 200., 100.));
            let blur = [Filter::Blur {
                std_deviation_x: 4.,
                std_deviation_y: 4.,
            }];
            let group = GroupRef {
                filters: &blur,
                ..GroupRef::new()
            };
            painter.with_group(group, |painter| {
                painter.fill_rect(Rect::new(10., 10., 20., 20.), color);
            });
            painter.pop_clip();
            scene
        };
        assert_eq!(
            diff(&make(RED), &make(BLUE)).unwrap(),
            Rect::new(0., 0., 200., 100.)
        );

        let mut copy = Scene::new();
        Painter::new(&mut copy)
            .fill(Rect::new(10., 10., 20., 20.), RED)
            .composite(Composite::new(
                BlendMode::new(Mix::Normal, Compose::Copy),
                1.,
            ))
            .draw();
        let bounds = scene_bounds(&copy).unwrap();
        assert!(bounds.contains_rect(Rect::new(-1e6, -1e6, 1e6, 1e6)));
    }

    #[test]
    fn strokes_and_transforms_are_bounded() {
        let mut scene = Scene::new();
        Painter::new(&mut scene)
            .stroke(Rect::new(0., 0., 10., 10.), &Stroke::new(4.), RED)
            .transform(Affine::translate((100., 50.)))
            .draw();
        let bounds = scene_bounds(&scene).unwrap();
        assert!(
            bounds.contains_rect(Rect::new(98., 48., 112., 62.)),
            "{bounds:?}"
        );
        assert!(bounds.width() < 20., "{bounds:?}");
    }

    #[test]
    fn rects_merge_when_overlapping_or_too_many() {
        let mut rects = Vec::new();
        push_rect(&mut rects, Rect::new(0., 0., 10., 10.), 3);
        push_rect(&mut rects, Rect::new(5., 5., 15., 15.), 3);
        assert_eq!(rects, [Rect::new(0., 0., 15., 15.)]);
        for x in [100., 200., 300.] {
            push_rect(&mut rects, Rect::new(x, 0., x + 10., 10.), 3);
        }
        assert_eq!(rects.len(), 3);
        let total = rects.iter().fold(Rect::ZERO, |a, b| a.union(*b));
        assert_eq!(total, Rect::new(0., 0., 310., 15.));
    }

    #[test]
    fn damage_bounds_clip_to_the_window() {
        let window = Rect::new(0., 0., 100., 100.);
        assert_eq!(Damage::Full.bounds(window), Some(window));
        assert_eq!(Damage::Region(vec![]).bounds(window), None);
        let damage = Damage::Region(vec![
            Rect::new(90., 90., 120., 120.),
            Rect::new(0., 0., 5., 5.),
        ]);
        assert_eq!(damage.bounds(window), Some(window));
        assert!(Damage::Region(vec![]).is_empty());
    }
}
