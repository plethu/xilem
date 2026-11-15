// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Damage regions reported by the paint pass, and damage-limited rendering.

use std::sync::Arc;

use masonry_imaging::vello_cpu::{IncrementalRenderer, Renderer, pack_xrgb, render_rgba8};
use masonry_imaging::{Layer, PreparedFrame};

use crate::app::{
    Damage, RenderRoot, RenderRootOptions, VisualLayerKind, VisualLayerPlan, WindowSizePolicy,
};
use crate::core::{
    ChildrenIds, CollectionWidget, NewWidget, Widget, WidgetPod, WidgetTag, WindowEvent,
};
use crate::dpi::PhysicalSize;
use crate::kurbo::{Point, Rect};
use crate::layout::{AsUnit, Length, SizeDef, UnitPoint};
use crate::palette::css::{BLUE, GREEN, ORANGE, PURPLE, RED};
use crate::peniko::Color;
use crate::properties::Background;
use crate::testing::{ModularWidget, ROBOTO, TestHarness, TestHarnessParams};
use crate::theme::test_property_set;
use crate::widgets::{Flex, IndexedStack, Label, Portal, SizedBox, TextArea, ZStack};

const BOX: Length = Length::const_px(40.);

fn colored_box(color: Color) -> NewWidget<SizedBox> {
    NewWidget::new(SizedBox::empty().width(BOX).height(BOX)).with_props(Background::Color(color))
}

fn damage<W: Widget>(harness: &mut TestHarness<W>) -> Damage {
    harness.redraw().0.damage
}

fn window_rect<W: Widget>(harness: &TestHarness<W>, tag: WidgetTag<impl Widget>) -> Rect {
    harness.get_widget(tag).ctx().bounding_box()
}

fn window_bounds<W: Widget>(harness: &mut TestHarness<W>) -> Rect {
    let size = harness.window_size();
    let scale = harness.render_root().scale_factor();
    Rect::new(
        0.,
        0.,
        f64::from(size.width) / scale,
        f64::from(size.height) / scale,
    )
}

#[track_caller]
fn assert_contains(outer: Rect, inner: Rect) {
    assert!(
        outer.x0 <= inner.x0
            && outer.y0 <= inner.y0
            && outer.x1 >= inner.x1
            && outer.y1 >= inner.y1,
        "{outer:?} does not contain {inner:?}"
    );
}

#[track_caller]
fn assert_region(damage: &Damage) -> &[Rect] {
    match damage {
        Damage::Full => panic!("expected a damage region, got full damage"),
        Damage::Region(rects) => rects,
    }
}

// --- MARK: FULL DAMAGE

#[test]
fn first_frame_is_full_and_idle_frames_are_empty() {
    let test_font = crate::peniko::Blob::new(Arc::new(ROBOTO));
    let mut root = RenderRoot::new(
        colored_box(RED).erased(),
        |_| {},
        RenderRootOptions {
            default_properties: Arc::new(test_property_set()),
            use_system_fonts: false,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(100, 100),
            scale_factor: 1.0,
            test_font: Some(test_font),
        },
    );
    assert_eq!(root.redraw().0.damage, Damage::Full);
    assert!(root.redraw().0.damage.is_empty());
}

#[test]
fn resize_scale_and_theme_changes_are_full() {
    let mut harness =
        TestHarness::create_with_size(test_property_set(), colored_box(RED), (200, 100));
    assert!(damage(&mut harness).is_empty());

    harness.process_window_event(WindowEvent::Resize(PhysicalSize::new(240, 120)));
    assert_eq!(damage(&mut harness), Damage::Full);
    assert!(damage(&mut harness).is_empty());

    harness.process_window_event(WindowEvent::Rescale(2.0));
    assert_eq!(damage(&mut harness), Damage::Full);
    assert!(damage(&mut harness).is_empty());

    harness.set_default_properties(Arc::new(test_property_set()));
    assert_eq!(damage(&mut harness), Damage::Full);
    assert!(damage(&mut harness).is_empty());
}

#[test]
fn damage_covering_most_of_the_window_is_full() {
    let tag = WidgetTag::unique();
    let big = NewWidget::new(SizedBox::empty().width(190.px()).height(90.px()))
        .with_props(Background::Color(RED))
        .with_tag(tag);
    let mut harness = TestHarness::create_with_size(test_property_set(), big, (200, 100));
    harness.edit_widget(tag, |mut widget| {
        widget.insert_prop(Background::Color(BLUE));
    });
    assert_eq!(damage(&mut harness), Damage::Full);
}

// --- MARK: REPAINT

fn three_boxes(tags: [WidgetTag<SizedBox>; 3]) -> NewWidget<Flex> {
    NewWidget::new(
        Flex::row()
            .with_fixed(colored_box(RED).with_tag(tags[0]))
            .with_fixed(colored_box(GREEN).with_tag(tags[1]))
            .with_fixed(colored_box(BLUE).with_tag(tags[2])),
    )
}

#[test]
fn repaint_damage_is_bounded_by_the_widget() {
    let tags = [
        WidgetTag::unique(),
        WidgetTag::unique(),
        WidgetTag::unique(),
    ];
    let mut harness =
        TestHarness::create_with_size(test_property_set(), three_boxes(tags), (300, 100));
    let target = window_rect(&harness, tags[1]);

    harness.edit_widget(tags[1], |mut widget| {
        widget.insert_prop(Background::Color(ORANGE));
    });
    let damage = damage(&mut harness);
    let rects = assert_region(&damage);
    assert!(!rects.is_empty());
    for rect in rects {
        assert_contains(target, *rect);
    }
    assert_contains(damage.bounds(target).unwrap(), target);
}

#[test]
fn identical_repaint_has_no_damage() {
    let tags = [
        WidgetTag::unique(),
        WidgetTag::unique(),
        WidgetTag::unique(),
    ];
    let mut harness =
        TestHarness::create_with_size(test_property_set(), three_boxes(tags), (300, 100));
    harness.edit_widget(tags[1], |mut widget| {
        widget.ctx.request_paint_only();
    });
    assert!(damage(&mut harness).is_empty());
}

#[test]
fn paint_outside_the_layout_box_is_damaged() {
    // The child paints 30px beyond its 10x10 border box without declaring paint insets.
    let tag = WidgetTag::unique();
    let child = NewWidget::new(
        ModularWidget::new(GREEN)
            .measure_fn(|_, _, _, _, _, _| 10.px())
            .paint_fn(|color, _, _, painter| {
                painter.fill_rect(Rect::new(-30., -30., 40., 40.), *color);
            }),
    )
    .with_tag(tag);
    let parent = NewWidget::new(Flex::row().with_fixed_spacer(60.px()).with_fixed(child));
    let mut harness = TestHarness::create_with_size(test_property_set(), parent, (200, 100));
    let border_box = window_rect(&harness, tag);

    harness.edit_widget(tag, |mut widget| {
        widget.widget.state = RED;
        widget.ctx.request_paint_only();
    });
    let damage = damage(&mut harness);
    let painted = Rect::new(-30., -30., 40., 40.) + border_box.origin().to_vec2();
    assert_contains(
        damage.bounds(window_bounds(&mut harness)).unwrap(),
        painted.intersect(window_bounds(&mut harness)),
    );
}

// --- MARK: GEOMETRY

struct ChildAndPos {
    child: WidgetPod<dyn Widget>,
    pos: Point,
}

fn positioned(child: NewWidget<impl Widget + ?Sized>) -> ModularWidget<ChildAndPos> {
    ModularWidget::new(ChildAndPos {
        child: child.erased().to_pod(),
        pos: Point::new(10., 10.),
    })
    .layout_fn(|state, ctx, _, size| {
        let child_size = ctx.compute_size(&mut state.child, SizeDef::fit(size), size.into());
        ctx.run_layout(&mut state.child, child_size);
        ctx.place_child(&mut state.child, state.pos);
    })
    .register_children_fn(|state, ctx| ctx.register_child(&mut state.child))
    .children_fn(|state| ChildrenIds::from_slice(&[state.child.id()]))
}

#[test]
fn moved_widget_damages_old_and_new_rects() {
    let child_tag = WidgetTag::unique();
    let parent_tag = WidgetTag::unique();
    let parent =
        NewWidget::new(positioned(colored_box(RED).with_tag(child_tag))).with_tag(parent_tag);
    let mut harness = TestHarness::create_with_size(test_property_set(), parent, (300, 200));
    let old = window_rect(&harness, child_tag);

    harness.edit_widget(parent_tag, |mut parent| {
        parent.widget.state.pos = Point::new(200., 120.);
        parent.ctx.request_layout();
    });
    let new = window_rect(&harness, child_tag);
    assert_ne!(old, new);

    let damage = damage(&mut harness);
    let rects = assert_region(&damage);
    assert!(
        rects.iter().any(|rect| rect.contains_rect(old)),
        "{rects:?}"
    );
    assert!(
        rects.iter().any(|rect| rect.contains_rect(new)),
        "{rects:?}"
    );
    // The space between them is not repainted.
    let between = Point::new(120., 90.);
    assert!(
        !rects.iter().any(|rect| rect.contains(between)),
        "{rects:?}"
    );
}

#[test]
fn scrolling_damages_the_viewport() {
    let portal_tag = WidgetTag::unique();
    let mut column = Flex::column();
    for i in 0..20 {
        let color = [RED, GREEN, BLUE, ORANGE][i % 4];
        column = column.with_fixed(colored_box(color));
    }
    let portal = NewWidget::new(Portal::new(NewWidget::new(column))).with_tag(portal_tag);
    let root = NewWidget::new(
        Flex::row()
            .with_fixed_spacer(30.px())
            .with_fixed(NewWidget::new(
                SizedBox::new(portal).width(80.px()).height(120.px()),
            )),
    );
    let mut harness = TestHarness::create_with_size(test_property_set(), root, (300, 200));
    let viewport = window_rect(&harness, portal_tag);

    harness.edit_widget(portal_tag, |mut portal| {
        Portal::set_viewport_pos(&mut portal, Point::new(0., 70.));
    });
    let damage = damage(&mut harness);
    let bounds = damage.bounds(window_bounds(&mut harness)).unwrap();
    assert_contains(viewport, bounds);
    // Content moved under the whole viewport.
    assert!(
        bounds.area() >= viewport.area() * 0.9,
        "{bounds:?} vs {viewport:?}"
    );
}

#[test]
fn removed_added_and_stashed_widgets_are_damaged() {
    let stack_tag = WidgetTag::unique();
    let first = WidgetTag::unique();
    let second = WidgetTag::unique();
    let stack = NewWidget::new(
        IndexedStack::new()
            .with(colored_box(RED).with_tag(first))
            .with(colored_box(BLUE).with_tag(second)),
    )
    .with_tag(stack_tag);
    let flex_tag = WidgetTag::unique();
    let removed = WidgetTag::unique();
    let flex = NewWidget::new(
        Flex::row()
            .with_fixed(stack)
            .with_fixed_spacer(100.px())
            .with_fixed(colored_box(GREEN).with_tag(removed)),
    )
    .with_tag(flex_tag);
    let mut harness = TestHarness::create_with_size(test_property_set(), flex, (300, 100));
    let shown = window_rect(&harness, first);

    // Switching the active child stashes one and unstashes the other at the same place.
    harness.edit_widget(stack_tag, |mut stack| {
        IndexedStack::set_active_child(&mut stack, 1);
    });
    let damage_after_switch = damage(&mut harness);
    let bounds = damage_after_switch
        .bounds(window_bounds(&mut harness))
        .unwrap();
    assert_contains(bounds, shown);

    let gone = window_rect(&harness, removed);
    harness.edit_widget(flex_tag, |mut flex| {
        Flex::remove(&mut flex, 2);
    });
    let damage_after_remove = damage(&mut harness);
    let bounds = damage_after_remove
        .bounds(window_bounds(&mut harness))
        .unwrap();
    assert_contains(bounds, gone);
    assert!(!bounds.contains(shown.center()));

    harness.edit_widget(flex_tag, |mut flex| {
        Flex::add_fixed(&mut flex, colored_box(PURPLE));
    });
    let damage_after_add = damage(&mut harness);
    let bounds = damage_after_add
        .bounds(window_bounds(&mut harness))
        .unwrap();
    assert_contains(bounds, gone);
}

#[test]
fn reordered_children_damage_their_parent() {
    let stack_tag = WidgetTag::unique();
    let small = NewWidget::new(SizedBox::empty().width(20.px()).height(20.px()))
        .with_props(Background::Color(BLUE));
    let stack = NewWidget::new(
        ZStack::new()
            .with(colored_box(RED), UnitPoint::CENTER)
            .with(small, UnitPoint::CENTER),
    )
    .with_tag(stack_tag);
    let mut harness = TestHarness::create_with_size(test_property_set(), stack, (200, 200));
    let stack_rect = window_rect(&harness, stack_tag);

    harness.edit_widget(stack_tag, |mut stack| {
        ZStack::swap(&mut stack, 0, 1);
    });
    let damage = damage(&mut harness);
    let bounds = damage.bounds(window_bounds(&mut harness)).unwrap();
    assert_contains(
        bounds,
        stack_rect.intersect(Rect::new(80., 80., 120., 120.)),
    );
}

#[test]
fn layers_damage_their_content() {
    let mut harness =
        TestHarness::create_with_size(test_property_set(), colored_box(RED), (300, 200));
    let layer = colored_box(GREEN);
    let layer_id = layer.id();
    harness
        .render_root()
        .add_layer(layer, Point::new(150., 100.));
    let added = damage(&mut harness);
    let bounds = added.bounds(window_bounds(&mut harness)).unwrap();
    assert_contains(bounds, Rect::new(150., 100., 190., 140.));
    assert!(!bounds.contains(Point::new(20., 20.)));

    harness
        .render_root()
        .reposition_layer(layer_id, Point::new(200., 20.));
    let moved = damage(&mut harness);
    let bounds = moved.bounds(window_bounds(&mut harness)).unwrap();
    assert_contains(bounds, Rect::new(150., 100., 190., 140.));
    assert_contains(bounds, Rect::new(200., 20., 240., 60.));

    harness.render_root().remove_layer(layer_id);
    let removed = damage(&mut harness);
    let bounds = removed.bounds(window_bounds(&mut harness)).unwrap();
    assert_contains(bounds, Rect::new(200., 20., 240., 60.));
}

// --- MARK: TEXT

#[test]
fn typing_and_caret_blink_damage_little() {
    let area_tag = WidgetTag::unique();
    let text = "The quick brown fox\njumps over the lazy dog\nand keeps on running\n";
    let area = NewWidget::new(TextArea::new_editable(&text.repeat(4))).with_tag(area_tag);
    let mut harness = TestHarness::create_with_size(
        test_property_set(),
        NewWidget::new(SizedBox::new(area).width(400.px()).height(300.px())),
        (400, 300),
    );
    let area_id = harness.get_widget(area_tag).id();
    harness.focus_on(Some(area_id));
    let _ = damage(&mut harness);
    let area_rect = window_rect(&harness, area_tag);

    harness.keyboard_type_chars("x");
    let typed = damage(&mut harness);
    let bounds = typed.bounds(area_rect).unwrap();
    assert_contains(area_rect, bounds);
    assert!(
        bounds.area() < area_rect.area() / 4.,
        "typing one character damaged {bounds:?} of {area_rect:?}"
    );

    // Let the caret blink off, then on.
    for _ in 0..4 {
        harness.animate_ms(600);
        let blink = damage(&mut harness);
        if let Some(bounds) = blink.bounds(area_rect) {
            assert!(bounds.width() < 40., "caret blink damaged {bounds:?}");
        }
    }
}

// --- MARK: PIXELS

/// Renders every frame both in full and limited to its damage, and compares the pixels.
struct PixelCheck {
    incremental: IncrementalRenderer,
    full: Renderer,
    pixels: Vec<u8>,
    partial_frames: usize,
}

impl PixelCheck {
    fn new() -> Self {
        Self {
            incremental: IncrementalRenderer::new().unwrap(),
            full: masonry_imaging::vello_cpu::new_headless_renderer().unwrap(),
            pixels: Vec::new(),
            partial_frames: 0,
        }
    }

    #[track_caller]
    fn frame<W: Widget>(&mut self, harness: &mut TestHarness<W>, step: &str) {
        let (plan, _) = harness.redraw();
        let size = harness.window_size();
        let scale = harness.render_root().scale_factor();
        self.compare(&plan, size, scale, step);
    }

    #[track_caller]
    fn compare(&mut self, plan: &VisualLayerPlan, size: PhysicalSize<u32>, scale: f64, step: &str) {
        let overlays: Vec<_> = plan
            .overlay_layers()
            .map(|layer| {
                let VisualLayerKind::Scene(scene) = &layer.kind else {
                    unreachable!()
                };
                Layer {
                    scene,
                    transform: layer.transform,
                }
            })
            .collect();
        let VisualLayerKind::Scene(root) = &plan.root_layer().unwrap().kind else {
            unreachable!()
        };
        let frame = PreparedFrame::new(
            size.width,
            size.height,
            scale,
            Color::WHITE,
            root,
            &overlays,
        );
        let updated = self.incremental.render(frame, plan.damage.rects()).unwrap();
        if updated.is_some() {
            self.partial_frames += 1;
        }
        render_rgba8(&mut self.full, frame, &mut self.pixels).unwrap();
        let expected = self.pixels.chunks_exact(4).map(pack_xrgb);
        for (i, (actual, expected)) in self
            .incremental
            .framebuffer()
            .iter()
            .zip(expected)
            .enumerate()
        {
            assert_eq!(
                *actual,
                expected,
                "{step}: pixel ({}, {}) is stale, damage {:?}",
                i as u32 % size.width,
                i as u32 / size.width,
                plan.damage
            );
        }
    }
}

#[test]
fn damage_limited_rendering_matches_full_rendering() {
    for scale in [1.0, 1.5] {
        let area_tag = WidgetTag::unique();
        let label_tag = WidgetTag::unique();
        let portal_tag = WidgetTag::unique();
        let box_tag = WidgetTag::unique();
        let stack_tag = WidgetTag::unique();
        let mut column = Flex::column();
        for i in 0..12 {
            column = column.with_fixed(NewWidget::new(Label::new(format!("Row {i}"))));
        }
        let portal = NewWidget::new(Portal::new(NewWidget::new(column))).with_tag(portal_tag);
        let stack = NewWidget::new(
            IndexedStack::new()
                .with(colored_box(RED))
                .with(colored_box(BLUE)),
        )
        .with_tag(stack_tag);
        let root = NewWidget::new(
            Flex::column()
                .with_fixed(NewWidget::new(Label::new("Status: ready")).with_tag(label_tag))
                .with_fixed(NewWidget::new(
                    Flex::row()
                        .with_fixed(NewWidget::new(
                            SizedBox::new(
                                NewWidget::new(TextArea::new_editable("Hello world\nSecond line"))
                                    .with_tag(area_tag),
                            )
                            .width(220.px())
                            .height(90.px()),
                        ))
                        .with_fixed(NewWidget::new(
                            SizedBox::new(portal).width(90.px()).height(90.px()),
                        ))
                        .with_fixed(stack),
                ))
                .with_fixed(colored_box(GREEN).with_tag(box_tag)),
        );
        let mut params = TestHarnessParams::default().with_size((400, 260));
        params.scale_factor = scale;
        let mut harness = TestHarness::create_with(test_property_set(), root, params);
        let mut check = PixelCheck::new();
        check.frame(&mut harness, "first frame");

        let area_id = harness.get_widget(area_tag).id();
        harness.focus_on(Some(area_id));
        check.frame(&mut harness, "focus");
        for (i, chars) in ["a", "b", "\n", "cd", " "].iter().enumerate() {
            harness.keyboard_type_chars(chars);
            check.frame(&mut harness, &format!("typing {i}"));
        }
        for i in 0..3 {
            harness.animate_ms(600);
            check.frame(&mut harness, &format!("caret blink {i}"));
        }
        harness.edit_widget(label_tag, |mut label| {
            Label::set_text(&mut label, "Status: 5 words");
        });
        check.frame(&mut harness, "label");
        for y in [30., 55., 0.] {
            harness.edit_widget(portal_tag, |mut portal| {
                Portal::set_viewport_pos(&mut portal, Point::new(0., y));
            });
            check.frame(&mut harness, &format!("scroll to {y}"));
        }
        let box_id = harness.get_widget(box_tag).id();
        harness.mouse_move_to(box_id);
        check.frame(&mut harness, "hover");
        harness.edit_widget(box_tag, |mut widget| {
            widget.insert_prop(Background::Color(PURPLE));
        });
        check.frame(&mut harness, "recolor");
        harness.edit_widget(stack_tag, |mut stack| {
            IndexedStack::set_active_child(&mut stack, 1);
        });
        check.frame(&mut harness, "stash");
        let layer = colored_box(ORANGE);
        let layer_id = layer.id();
        harness
            .render_root()
            .add_layer(layer, Point::new(300., 150.));
        check.frame(&mut harness, "add layer");
        harness
            .render_root()
            .reposition_layer(layer_id, Point::new(320., 40.));
        check.frame(&mut harness, "move layer");
        harness.render_root().remove_layer(layer_id);
        check.frame(&mut harness, "remove layer");
        harness.edit_widget(box_tag, |mut widget| {
            SizedBox::set_width(&mut widget, 80.px());
        });
        check.frame(&mut harness, "resize widget");

        assert!(
            check.partial_frames > 10,
            "only {} frames rendered partially",
            check.partial_frames
        );
    }
}
