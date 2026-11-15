// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use kurbo::{Affine, Line, Rect, Stroke};
use peniko::{Color, Fill};
use tracing::{info_span, trace};
use tree_arena::ArenaMut;

use crate::app::{
    DamageTracker, PaintGeometry, RenderRoot, RenderRootState, SLOTS, SlotChange, VisualLayer,
    VisualLayerKind, VisualLayerPlan, clip_to, scene_bounds,
};
use crate::core::{
    DefaultProperties, PaintCtx, PaintLayerMode, PropertiesRef, PropertyArena, WidgetArenaNode,
    WidgetId,
};
use crate::imaging::record::{Clip, Geometry, Scene};
use crate::imaging::{PaintSink, Painter};
use crate::passes::{enter_span_if, recurse_on_children};
use crate::util::get_debug_color;

struct LayerCollector {
    current_scene: Scene,
    layers: Vec<VisualLayer>,
    current_owner_id: WidgetId,
    transform: Affine,
}

impl LayerCollector {
    fn new(root_id: WidgetId, transform: Affine) -> Self {
        Self {
            current_scene: Scene::new(),
            layers: Vec::new(),
            current_owner_id: root_id,
            transform,
        }
    }

    fn scene_mut(&mut self) -> &mut Scene {
        &mut self.current_scene
    }

    fn finish_current_layer(&mut self, allow_empty: bool) {
        let empty_scene = Scene::new();
        if !allow_empty && self.current_scene == empty_scene {
            return;
        }

        let scene = std::mem::replace(&mut self.current_scene, empty_scene);
        self.layers.push(VisualLayer {
            kind: VisualLayerKind::Scene(scene),
            transform: self.transform,
            widget_id: self.current_owner_id,
        });
    }

    fn push_external_layer(&mut self, widget_id: WidgetId, bounds: Rect) {
        self.layers.push(VisualLayer {
            kind: VisualLayerKind::External { bounds },
            transform: self.transform,
            widget_id,
        });
    }

    fn into_layers(mut self) -> Vec<VisualLayer> {
        self.finish_current_layer(self.layers.is_empty());
        self.layers
    }
}

/// Per-layer state shared by every widget painted into it.
struct LayerContext<'a> {
    default_properties: &'a DefaultProperties,
    property_arena: &'a PropertyArena,
    window_to_layer_transform: Affine,
    /// Transform the host applies to the layer's scene; maps layer space back to window space.
    layer_to_window_transform: Affine,
}

/// Repaints one scene slot, diffing its old and new contents.
fn repaint_slot(
    damage: &mut DamageTracker,
    scene: &mut Scene,
    paint: impl FnOnce(&mut Painter<'_>),
) -> SlotChange {
    // The previous contents move to the scratch scene for diffing; the slot keeps the scratch
    // scene's allocations.
    std::mem::swap(scene, &mut damage.scratch);
    scene.clear();
    {
        let sink_dyn: &mut dyn PaintSink = scene;
        let mut painter = Painter::new(sink_dyn);
        paint(&mut painter);
    }
    damage.diff_slot(scene)
}

// --- MARK: PAINT WIDGET
/// Paints a widget and its descendants, returning the window-space rectangle they cover.
///
/// `clip` is the window-space bounds of ancestor clips.
fn paint_widget(
    global_state: &mut RenderRootState,
    layer: &LayerContext<'_>,
    layer_collector: &mut LayerCollector,
    scene_cache: &mut HashMap<WidgetId, (Scene, Scene, Scene)>,
    damage: &mut DamageTracker,
    clip: Rect,
    node: ArenaMut<'_, WidgetArenaNode>,
) -> Option<Rect> {
    let default_properties = layer.default_properties;
    let property_arena = layer.property_arena;
    let window_to_layer_transform = &layer.window_to_layer_transform;
    let mut children = node.children;
    let widget = &mut *node.item.widget;
    let state = &mut node.item.state;
    let properties = &mut node.item.properties;
    let class_set = &node.item.class_set;
    let id = state.id;

    let trace = global_state.trace.paint;
    let _span = enter_span_if(trace, state);

    // Note: At this point we could short-circuit if is_stashed is true,
    // but we deliberately avoid doing that to avoid creating zombie flags.
    // (See WidgetState doc.)
    let is_stashed = state.is_stashed;

    let mut changes: [Option<SlotChange>; SLOTS] = [None; SLOTS];
    state.paint_layer_mode = PaintLayerMode::Inline;
    if (state.request_pre_paint || state.request_paint || state.request_post_paint) && !is_stashed {
        if trace {
            trace!("Painting widget '{}' {}", widget.short_type_name(), id);
        }

        let stack = property_arena.get(state.property_stack_id, widget.type_id());
        let mut ctx = PaintCtx {
            global_state,
            widget_state: state,
            children: children.reborrow_mut(),
        };
        let props = PropertiesRef {
            local: properties,
            default_map: default_properties.for_widget(widget.type_id()),
            stack,
            class_set,
        };

        // TODO - Reserve scene
        // https://github.com/linebender/xilem/issues/524
        let (pre_scene, scene, post_scene) = scene_cache.entry(id).or_default();
        if ctx.widget_state.request_pre_paint {
            changes[0] = Some(repaint_slot(damage, pre_scene, |painter| {
                widget.pre_paint(&mut ctx, &props, painter);
            }));
        }
        if ctx.widget_state.request_paint {
            changes[1] = Some(repaint_slot(damage, scene, |painter| {
                widget.paint(&mut ctx, &props, painter);
            }));
        }
        if ctx.widget_state.request_post_paint {
            changes[2] = Some(repaint_slot(damage, post_scene, |painter| {
                widget.post_paint(&mut ctx, &props, painter);
            }));
        }
    }

    state.request_pre_paint = false;
    state.request_paint = false;
    state.request_post_paint = false;
    state.needs_paint = false;

    let paint_layer_mode = if is_stashed {
        PaintLayerMode::Inline
    } else {
        state.paint_layer_mode
    };

    if matches!(
        paint_layer_mode,
        PaintLayerMode::IsolatedScene | PaintLayerMode::External
    ) {
        layer_collector.finish_current_layer(false);
    }

    let previous_owner_id = layer_collector.current_owner_id;
    layer_collector.current_owner_id = match paint_layer_mode {
        PaintLayerMode::Inline => previous_owner_id,
        PaintLayerMode::IsolatedScene | PaintLayerMode::External => id,
    };

    let border_box_to_layer_transform = *window_to_layer_transform * state.window_transform;
    let content_box_to_layer_transform =
        border_box_to_layer_transform.pre_translate(state.border_box_translation());
    let has_clip = state.clip_path.is_some();
    let paint_as_external = paint_layer_mode == PaintLayerMode::External;

    // Damage is tracked in window space, through the same transforms the host applies.
    let border_box_to_window_transform =
        layer.layer_to_window_transform * border_box_to_layer_transform;
    let inner_clip = match state.clip_path {
        Some(clip_path) => clip_to(
            border_box_to_window_transform.transform_rect_bbox(clip_path),
            clip,
        )
        .unwrap_or(Rect::ZERO),
        None => clip,
    };
    let geometry = PaintGeometry {
        transform: border_box_to_window_transform.pre_translate(state.border_box_translation()),
        outer_clip: clip,
        inner_clip,
        painted: !is_stashed && !paint_as_external,
        external: (paint_as_external && !is_stashed)
            .then(|| border_box_to_window_transform.transform_rect_bbox(state.border_box())),
    };
    let own_rect = damage.update_widget(&mut state.paint_record, &geometry, changes, |slot| {
        let (pre, scene, post) = scene_cache.get(&id)?;
        scene_bounds([pre, scene, post][slot])
    });

    if !is_stashed && !paint_as_external {
        let Some((pre_scene, scene, _)) = &mut scene_cache.get(&id) else {
            debug_panic!(
                "Error in paint pass: scene should have been cached earlier in this function."
            );
            return None;
        };

        layer_collector
            .scene_mut()
            .append_transformed(pre_scene, content_box_to_layer_transform);

        if let Some(clip) = state.clip_path {
            // The clip path is stored in border-box space, so need to use that transform.
            layer_collector.scene_mut().push_clip(Clip::Fill {
                transform: border_box_to_layer_transform,
                shape: Geometry::Rect(clip),
                fill_rule: Fill::NonZero,
            });
        }

        layer_collector
            .scene_mut()
            .append_transformed(scene, content_box_to_layer_transform);
    }

    // The widget's clip only applies to children when its scenes are painted inline.
    let child_clip = if geometry.painted { inner_clip } else { clip };
    let parent_state = &mut *state;
    let mut subtree = own_rect;
    let mut children_hash = 0_u64;
    recurse_on_children(id, widget, children, |mut node| {
        // TODO: We could skip painting children outside the parent clip path.
        // There's a few things to consider if we do:
        // - Some widgets can paint outside of their border-box.
        // - Once we implement compositor layers, we may want to paint outside of the clip path anyway in anticipation of user scrolling.
        // - We still want to reset needs_paint and request_paint flags.
        children_hash = (children_hash.rotate_left(5) ^ node.item.state.id.to_raw())
            .wrapping_mul(0x517c_c1b7_2722_0a95);
        let child_rect = paint_widget(
            global_state,
            layer,
            layer_collector,
            scene_cache,
            damage,
            child_clip,
            node.reborrow_mut(),
        );
        if let Some(child_rect) = child_rect {
            subtree = Some(subtree.map_or(child_rect, |rect| rect.union(child_rect)));
        }
        parent_state.merge_up(&mut node.item.state);
    });
    damage.update_subtree(
        &mut parent_state.paint_record,
        subtree,
        children_hash,
        || widget.children_ids().into_iter().collect(),
    );

    if !is_stashed && !paint_as_external {
        if global_state.debug_paint {
            // Draw the global axis aligned bounding rect of the widget
            const BORDER_WIDTH: f64 = 1.0;
            let color = get_debug_color(id.to_raw());
            let rect = state.bounding_box.inset(BORDER_WIDTH / -2.0);
            let border_style = Stroke::new(BORDER_WIDTH);
            let mut painter = Painter::new(layer_collector.scene_mut());
            painter.stroke(rect, &border_style, color).draw();

            // Draw the widget's explicit baselines
            let mut draw_baseline = |baseline| {
                let border_box = state.border_box();
                let line = Line::new((border_box.x0, baseline), (border_box.x1, baseline));
                let baseline_style = Stroke::new(1.0).with_dashes(0., [4.0, 4.0]);
                painter
                    .stroke(line, &baseline_style, color)
                    .transform(border_box_to_layer_transform)
                    .draw();
            };
            if !state.first_baseline.is_nan() {
                draw_baseline(state.first_baseline);
            }
            if !state.last_baseline.is_nan() && state.last_baseline != state.first_baseline {
                draw_baseline(state.last_baseline);
            }
        }

        if has_clip {
            layer_collector.scene_mut().pop_clip();
        }

        let Some((_, _, post_scene)) = &mut scene_cache.get(&id) else {
            debug_panic!(
                "Error in paint pass: scene should have been cached earlier in this function."
            );
            return None;
        };

        layer_collector
            .scene_mut()
            .append_transformed(post_scene, content_box_to_layer_transform);

        if global_state.inspector_state.hovered_widget == Some(id) {
            const HOVER_FILL_COLOR: Color = Color::from_rgba8(60, 60, 250, 100);
            let rect = state.border_box();
            Painter::new(layer_collector.scene_mut())
                .fill(rect, HOVER_FILL_COLOR)
                .transform(border_box_to_layer_transform)
                .draw();
        }
    }

    if paint_as_external {
        layer_collector.push_external_layer(id, state.border_box());
    }

    if matches!(
        paint_layer_mode,
        PaintLayerMode::IsolatedScene | PaintLayerMode::External
    ) {
        layer_collector.finish_current_layer(false);
    }

    layer_collector.current_owner_id = previous_owner_id;
    subtree
}

// --- MARK: ROOT
/// See the [passes documentation](crate::doc::pass_system#render-passes).
pub(crate) fn run_paint_pass(root: &mut RenderRoot) -> VisualLayerPlan {
    let _span = info_span!("paint").entered();

    // TODO - This is a bit of a hack until we refactor widget tree mutation.
    // This should be removed once remove_child is exclusive to MutateCtx.
    let mut scene_cache = std::mem::take(&mut root.global_state.scene_cache);

    let mut damage = std::mem::take(&mut root.global_state.damage);

    let root_id = root.root_id();
    let layer_root_ids = root.layer_root_ids();

    // Debug paint and the inspector overlay draw outside widget scenes.
    let overlays_active =
        root.global_state.debug_paint || root.global_state.inspector_state.hovered_widget.is_some();
    let window = Rect::from_origin_size(kurbo::Point::ORIGIN, root.get_kurbo_size());
    damage.begin_frame(
        root.global_state.size,
        root.global_state.scale_factor,
        window,
        &layer_root_ids,
        overlays_active,
    );

    let mut layers = Vec::new();

    for (idx, &layer_widget_id) in layer_root_ids.iter().enumerate() {
        let transform = if idx == 0 {
            Affine::IDENTITY
        } else {
            root.widget_arena
                .get_state(layer_widget_id)
                .window_transform
        };

        let mut collector = LayerCollector::new(layer_widget_id, transform);
        paint_layer(
            root,
            &mut collector,
            &mut scene_cache,
            &mut damage,
            window,
            root_id,
            layer_widget_id,
        );
        layers.extend(collector.into_layers());
    }

    root.global_state.scene_cache = scene_cache;
    let damage_region = damage.finish_frame();
    root.global_state.damage = damage;

    VisualLayerPlan {
        layers,
        damage: damage_region,
    }
}

/// Paint a single layer's widget subtree into `target_scene`.
///
/// This is a helper that handles the split borrows needed to access
/// `global_state`, `default_properties`, and `widget_arena` simultaneously.
fn paint_layer(
    root: &mut RenderRoot,
    layer_collector: &mut LayerCollector,
    scene_cache: &mut HashMap<WidgetId, (Scene, Scene, Scene)>,
    damage: &mut DamageTracker,
    window: Rect,
    root_id: WidgetId,
    layer_widget_id: WidgetId,
) {
    // Clear the LayerStack's own paint flags (its paint is a no-op).
    // This is idempotent so safe to call per-layer.
    {
        let root_node = root.widget_arena.get_node_mut(root_id);
        let state = &mut root_node.item.state;
        state.request_pre_paint = false;
        state.request_paint = false;
        state.request_post_paint = false;
        state.needs_paint = false;
    }

    // Get the layer child from the arena, then pass split borrows to paint_widget.
    let root_node = root.widget_arena.get_node_mut(root_id);
    let Some(layer_node) = root_node.children.into_item_mut(layer_widget_id) else {
        debug_panic!(
            "Error in paint pass: cannot find layer child {layer_widget_id:?} in LayerStack"
        );
        return;
    };

    let window_to_layer_transform = layer_node.item.state.window_transform.inverse();
    let layer = LayerContext {
        default_properties: &root.property_arena.default_properties,
        property_arena: &root.property_arena,
        window_to_layer_transform,
        layer_to_window_transform: layer_collector.transform,
    };

    paint_widget(
        &mut root.global_state,
        &layer,
        layer_collector,
        scene_cache,
        damage,
        window,
        layer_node,
    );
}
