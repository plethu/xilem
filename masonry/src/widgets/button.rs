// Copyright 2018 the Xilem Authors and the Druid Authors
// SPDX-License-Identifier: Apache-2.0

use std::any::TypeId;
use std::sync::Arc;

use accesskit::{Node, Role, Toggled};
use include_doc_path::include_doc_path;
use tracing::{Span, trace, trace_span};

use crate::core::MeasureCtx;
use crate::core::keyboard::{Key, NamedKey};
use crate::core::pointer::PointerButton;
use crate::core::{
    AccessCtx, AccessEvent, ArcStr, ChildrenIds, EventCtx, LayoutCtx, NewWidget, PaintCtx,
    PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update,
    UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use crate::imaging::Painter;
use crate::kurbo::{Axis, Size};
use crate::layout::{LayoutSize, LenReq, Length, SizeDef};
use crate::theme;
use crate::widgets::Label;

/// A button with a child widget.
///
/// Emits [`ButtonPress`] when pressed.
///
#[doc = concat!(
    "![Button with text label](",
    include_doc_path!("screenshots/button_hello.png"),
    ")",
)]
pub struct Button {
    child: WidgetPod<dyn Widget>,
    access_state: ButtonAccessState,
    role: Role,
    accessible_label: Option<ArcStr>,
    accessible_description: Option<ArcStr>,
    focusable: bool,
}

/// Accessibility states a [`Button`] reports on its node.
///
/// A `None` field leaves that state off the node, so assistive technology doesn't announce it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonAccessState {
    /// Whether the content this button controls is expanded (`true`) or collapsed (`false`).
    ///
    /// Use this for disclosure buttons instead of putting arrows in the button's name.
    pub expanded: Option<bool>,
    /// Whether this button is the selected item, for example in a picker, list or tab row.
    pub selected: Option<bool>,
    /// Whether this toggle button is pressed.
    ///
    /// Any value makes platforms expose the button as a toggle button.
    pub toggled: Option<Toggled>,
}

// --- MARK: BUILDERS
impl Button {
    /// Creates a new button with a child widget.
    ///
    /// The child widget probably shouldn't be interactive,
    /// to avoid behaviour which might be confusing to the user.
    ///
    /// # Examples
    ///
    /// ```
    /// use masonry::widgets::{Button, Label};
    /// use masonry::core::Widget;
    ///
    /// let button = Button::new(Label::new("Increment").prepare());
    /// ```
    pub fn new(child: NewWidget<impl Widget + ?Sized>) -> Self {
        Self {
            child: child.erased().to_pod(),
            access_state: ButtonAccessState::default(),
            role: Role::Button,
            accessible_label: None,
            accessible_description: None,
            focusable: true,
        }
    }

    /// Creates a new button with a label widget.
    ///
    /// # Examples
    ///
    /// ```
    /// use masonry::widgets::Button;
    /// use masonry::core::Widget;
    ///
    /// let button = Button::with_text("Increment");
    /// ```
    pub fn with_text(text: impl Into<Arc<str>>) -> Self {
        Self::new(Label::new(text).prepare())
    }

    /// Builder-style method to set the expanded, selected and toggled states
    /// this button reports to assistive technology.
    ///
    /// The default reports none of them.
    pub fn with_access_state(mut self, access_state: ButtonAccessState) -> Self {
        self.access_state = access_state;
        self
    }

    /// Returns the expanded, selected and toggled states this button reports.
    pub fn access_state(&self) -> ButtonAccessState {
        self.access_state
    }

    /// Builder-style method to set the accessibility role this button reports.
    ///
    /// The default is [`Role::Button`]. Use [`Role::Tab`] with a `selected`
    /// [access state](ButtonAccessState) for a tab inside a [`Role::TabList`] container.
    /// The role only changes what assistive technology is told; activation is unchanged.
    ///
    /// AccessKit names a [`Role::Button`] from its descendant labels, but not a [`Role::Tab`];
    /// give such a button an [accessible label](Self::with_accessible_label).
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Builder-style method to set the name assistive technology announces for this button.
    ///
    /// The default is no explicit name, so a [`Role::Button`] is named from its descendant
    /// labels. An explicit name replaces that.
    pub fn with_accessible_label(mut self, label: impl Into<ArcStr>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    /// Builder-style method to set the description assistive technology reads after the
    /// button's name, such as why it is unavailable.
    pub fn with_accessible_description(mut self, description: impl Into<ArcStr>) -> Self {
        self.accessible_description = Some(description.into());
        self
    }

    /// Builder-style method to set whether Tab and Shift+Tab can focus this button.
    ///
    /// The default is `true`. A button that isn't focusable behaves like an HTML element with
    /// `tabindex="-1"`: Tab skips it, but pointer presses, [`EventCtx::request_focus`],
    /// [`EventCtx::set_focus`] and the AccessKit `Focus` action can still focus it, and it
    /// still activates from pointer clicks, keys while focused and the AccessKit `Click` action.
    /// This supports roving focus, where only the selected tab of a tab list is a Tab stop.
    pub fn with_focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// Returns the accessibility role this button reports.
    pub fn role(&self) -> Role {
        self.role
    }
}

// --- MARK: WIDGETMUT
impl Button {
    /// Replaces the child widget with a new one.
    pub fn set_child(this: &mut WidgetMut<'_, Self>, child: NewWidget<impl Widget + ?Sized>) {
        this.ctx.remove_child(std::mem::replace(
            &mut this.widget.child,
            child.erased().to_pod(),
        ));
    }

    /// Returns a mutable reference to the child.
    pub fn child_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }

    /// Sets the expanded, selected and toggled states this button reports.
    ///
    /// See [`Button::with_access_state`] for more details.
    pub fn set_access_state(this: &mut WidgetMut<'_, Self>, access_state: ButtonAccessState) {
        if this.widget.access_state != access_state {
            this.widget.access_state = access_state;
            this.ctx.request_accessibility_update();
        }
    }

    /// Sets the accessibility role this button reports.
    ///
    /// See [`Button::with_role`] for more details.
    pub fn set_role(this: &mut WidgetMut<'_, Self>, role: Role) {
        if this.widget.role != role {
            this.widget.role = role;
            this.ctx.request_accessibility_update();
        }
    }

    /// Sets the name assistive technology announces for this button.
    ///
    /// See [`Button::with_accessible_label`] for more details.
    pub fn set_accessible_label(this: &mut WidgetMut<'_, Self>, label: Option<impl Into<ArcStr>>) {
        this.widget.accessible_label = label.map(Into::into);
        this.ctx.request_accessibility_update();
    }

    /// Sets the description assistive technology reads after the button's name.
    ///
    /// See [`Button::with_accessible_description`] for more details.
    pub fn set_accessible_description(
        this: &mut WidgetMut<'_, Self>,
        description: Option<impl Into<ArcStr>>,
    ) {
        this.widget.accessible_description = description.map(Into::into);
        this.ctx.request_accessibility_update();
    }

    /// Sets whether Tab and Shift+Tab can focus this button.
    ///
    /// See [`Button::with_focusable`] for more details. A focused button keeps focus when
    /// this turns off, so a roving-focus tab list can make the newly focused tab focusable
    /// and the previous one unfocusable in any order.
    pub fn set_focusable(this: &mut WidgetMut<'_, Self>, focusable: bool) {
        if this.widget.focusable != focusable {
            this.widget.focusable = focusable;
            this.ctx.accepts_focus_changed();
        }
    }
}

/// A button was pressed.
#[derive(PartialEq, Debug)]
pub struct ButtonPress {
    /// The pointer button that has been pressed.
    ///
    /// Can be `None` when using for example the keyboard or a touch screen.
    pub button: Option<PointerButton>,
}

// --- MARK: IMPL WIDGET
impl Widget for Button {
    type Action = ButtonPress;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(..) => {
                ctx.request_focus();
                ctx.capture_pointer();
                // Changes in pointer capture impact appearance, but not accessibility node
                ctx.request_paint_only();
                trace!("Button {:?} pressed", ctx.widget_id());
            }
            PointerEvent::Up(PointerButtonEvent { button, .. }) => {
                if ctx.is_active() && ctx.is_hovered() {
                    ctx.submit_action::<Self::Action>(ButtonPress { button: *button });
                    trace!("Button {:?} released", ctx.widget_id());
                }
                // Changes in pointer capture impact appearance, but not accessibility node
                ctx.request_paint_only();
            }
            _ => (),
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        match event {
            TextEvent::Keyboard(event)
                if event.state.is_up()
                    && (matches!(&event.key, Key::Character(c) if c == " ")
                        || event.key == Key::Named(NamedKey::Enter)) =>
            {
                ctx.submit_action::<Self::Action>(ButtonPress { button: None });
            }
            _ => (),
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        match event.action {
            accesskit::Action::Click => {
                ctx.submit_action::<Self::Action>(ButtonPress { button: None });
            }
            _ => {}
        }
    }

    fn update(
        &mut self,
        _ctx: &mut UpdateCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        _event: &Update,
    ) {
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn property_changed(&mut self, _ctx: &mut UpdateCtx<'_>, _property_type: TypeId) {}

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        cross_length: Option<Length>,
    ) -> Length {
        let auto_length = len_req.into();
        let context_size = LayoutSize::maybe(axis.cross(), cross_length);

        let child_length = ctx.compute_length(
            &mut self.child,
            auto_length,
            context_size,
            axis,
            cross_length,
        );

        let length = child_length;

        // TODO - Add MinimumSize property.
        // HACK: to make sure we look okay at default sizes when beside a text input,
        // we make sure we will have at least the same height as the default text input.
        match axis {
            Axis::Horizontal => length,
            Axis::Vertical => length.max(theme::BASIC_WIDGET_HEIGHT),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let child_size = ctx.compute_size(&mut self.child, SizeDef::fit(size), size.into());
        ctx.run_layout(&mut self.child, child_size);

        let child_origin = ((size - child_size).to_vec2() * 0.5).to_point();
        ctx.place_child(&mut self.child, child_origin);

        ctx.derive_baselines(&self.child);
    }

    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        _painter: &mut Painter<'_>,
    ) {
    }

    fn accessibility_role(&self) -> Role {
        self.role
    }

    fn accessibility(
        &mut self,
        ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.add_action(accesskit::Action::Click);
        // Masonry only advertises `Focus` for widgets in the Tab order, but a button that
        // isn't a Tab stop still takes focus, like `tabindex="-1"`.
        if !self.focusable && !ctx.is_disabled() {
            node.add_action(accesskit::Action::Focus);
        }
        if let Some(label) = &self.accessible_label {
            node.set_label(&**label);
        }
        if let Some(description) = &self.accessible_description {
            node.set_description(&**description);
        }

        let ButtonAccessState {
            expanded,
            selected,
            toggled,
        } = self.access_state;
        if let Some(expanded) = expanded {
            node.set_expanded(expanded);
        }
        if let Some(selected) = selected {
            node.set_selected(selected);
        }
        if let Some(toggled) = toggled {
            node.set_toggled(toggled);
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }

    fn propagates_pointer_interaction(&self) -> bool {
        false
    }

    fn accepts_focus(&self) -> bool {
        // Buttons can be tab-focused, unless they opted out.
        self.focusable
    }

    fn accepts_text_input(&self) -> bool {
        // But they still aren't text areas.
        false
    }

    fn make_trace_span(&self, id: WidgetId) -> Span {
        trace_span!("Button", id = id.trace())
    }
}

// --- MARK: TESTS
#[cfg(test)]
mod tests {
    use masonry_core::core::WidgetTag;
    use masonry_testing::{TestHarnessParams, assert_failing_render_snapshot};
    use std::assert_matches;

    use super::*;
    use crate::core::{CollectionWidget, PointerButton, PropertySet, StyleProperty};
    use crate::layout::AsUnit;
    use crate::properties::{
        BorderColor, BorderWidth, BoxShadow, ContentColor, CornerRadius, Gap, Padding,
    };
    use crate::testing::{TestHarness, assert_render_snapshot};
    use crate::theme::{ACCENT_COLOR, test_property_set};
    use crate::widgets::{Flex, Grid, GridParams, Label, SizedBox};

    #[test]
    fn simple_button() {
        let widget = NewWidget::new(Button::with_text("Hello"));

        let params =
            TestHarnessParams::size_and_padding((100, 40), TestHarnessParams::ROOT_PADDING);
        let mut harness = TestHarness::create_with(test_property_set(), widget, params);
        let button_id = harness.root_id();

        assert_render_snapshot!(harness, "button_hello");

        assert!(harness.pop_action_erased().is_none());

        harness.mouse_click_on(button_id, Some(PointerButton::Primary));
        assert_eq!(
            harness.pop_action::<ButtonPress>(),
            Some((
                ButtonPress {
                    button: Some(PointerButton::Primary)
                },
                button_id
            ))
        );

        // Check that Tab focuses on the widget
        harness.focus_on(None);
        harness.press_tab_key(false);
        assert_eq!(harness.focused_widget().map(|w| w.id()), Some(button_id));

        harness.process_text_event(TextEvent::key_down(Key::Character(" ".into())));
        harness.process_text_event(TextEvent::key_up(Key::Character(" ".into())));
        assert_eq!(
            harness.pop_action(),
            Some((ButtonPress { button: None }, button_id))
        );
    }

    #[test]
    fn mouse_down_requests_focus() {
        let widget = NewWidget::new(Button::with_text("Hello"));
        let mut harness = TestHarness::create(test_property_set(), widget);
        let button_id = harness.root_id();

        harness.focus_on(None);
        harness.mouse_move_to(button_id);
        harness.mouse_button_press(None);

        assert_eq!(harness.focused_widget_id(), Some(button_id));
    }

    #[test]
    fn access_state() {
        let widget = NewWidget::new(Button::with_text("Pin reference"));
        let mut harness = TestHarness::create(test_property_set(), widget);
        let button_id = harness.root_id();
        let _ = harness.render();

        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.data().is_expanded(), None);
        assert_eq!(node.data().is_selected(), None);
        assert_eq!(node.data().toggled(), None);

        harness.edit_root_widget(|mut button| {
            Button::set_access_state(
                &mut button,
                ButtonAccessState {
                    expanded: Some(false),
                    selected: Some(true),
                    toggled: Some(Toggled::True),
                },
            );
        });
        let _ = harness.render();

        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.data().is_expanded(), Some(false));
        assert_eq!(node.data().is_selected(), Some(true));
        assert_eq!(node.data().toggled(), Some(Toggled::True));

        harness.edit_root_widget(|mut button| {
            Button::set_access_state(&mut button, ButtonAccessState::default());
        });
        let _ = harness.render();

        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.data().is_expanded(), None);
        assert_eq!(node.data().is_selected(), None);
        assert_eq!(node.data().toggled(), None);
    }

    #[test]
    fn access_state_on_mount() {
        let state = ButtonAccessState {
            expanded: Some(true),
            ..ButtonAccessState::default()
        };
        let widget = NewWidget::new(Button::with_text("Details").with_access_state(state));
        let mut harness = TestHarness::create(test_property_set(), widget);
        let button_id = harness.root_id();
        let _ = harness.render();

        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.data().is_expanded(), Some(true));
        assert_eq!(node.data().is_selected(), None);
    }

    #[test]
    fn tab_role_and_label() {
        let state = ButtonAccessState {
            selected: Some(true),
            ..ButtonAccessState::default()
        };
        let widget = NewWidget::new(
            Button::with_text("Outline")
                .with_role(Role::Tab)
                .with_access_state(state),
        );
        let mut harness = TestHarness::create(test_property_set(), widget);
        let button_id = harness.root_id();
        let _ = harness.render();

        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.role(), Role::Tab);
        assert_eq!(node.data().is_selected(), Some(true));
        // AccessKit names buttons, but not tabs, from their descendant labels.
        assert_eq!(node.label(), None);

        harness.edit_root_widget(|mut button| {
            Button::set_accessible_label(&mut button, Some("Outline"));
        });
        let _ = harness.render();
        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.label().as_deref(), Some("Outline"));

        harness.edit_root_widget(|mut button| {
            Button::set_role(&mut button, Role::Button);
            Button::set_accessible_label(&mut button, None::<ArcStr>);
        });
        let _ = harness.render();
        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.label().as_deref(), Some("Outline"));
        assert_eq!(node.description(), None);

        harness.edit_root_widget(|mut button| {
            Button::set_accessible_description(&mut button, Some("Choose a beat first"));
        });
        let _ = harness.render();
        let node = harness.access_node(button_id).unwrap();
        assert_eq!(node.description().as_deref(), Some("Choose a beat first"));
    }

    #[test]
    fn unfocusable_buttons_support_roving_focus() {
        let tags = [
            WidgetTag::named("first"),
            WidgetTag::named("selected"),
            WidgetTag::named("third"),
        ];
        let after = WidgetTag::named("after");
        let tab = |text: &str, tag, focusable| {
            NewWidget::new(Button::with_text(text).with_focusable(focusable)).with_tag(tag)
        };
        let widget = Flex::row()
            .with_fixed(tab("First", tags[0], false))
            .with_fixed(tab("Selected", tags[1], true))
            .with_fixed(tab("Third", tags[2], false))
            .with_fixed(NewWidget::new(Button::with_text("After")).with_tag(after))
            .prepare();
        let mut harness = TestHarness::create(test_property_set(), widget);
        let [first, selected, third] = tags.map(|tag| harness.get_widget(tag).id());
        let after = harness.get_widget(after).id();
        let _ = harness.render();

        // Only the focusable tab is a Tab stop, but every tab advertises `Focus`.
        harness.press_tab_key(false);
        assert_eq!(harness.focused_widget_id(), Some(selected));
        harness.press_tab_key(false);
        assert_eq!(harness.focused_widget_id(), Some(after));
        for id in [first, selected, third] {
            let node = harness.access_node(id).unwrap();
            assert!(node.data().supports_action(accesskit::Action::Focus));
        }

        // Programmatic and AccessKit focus reach a tab that isn't a Tab stop.
        harness.focus_on(Some(first));
        assert_eq!(harness.focused_widget_id(), Some(first));
        harness.process_access_event(accesskit::ActionRequest {
            action: accesskit::Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: third.to_raw().into(),
            data: None,
        });
        assert_eq!(harness.focused_widget_id(), Some(third));

        // Keys, pointer clicks and AccessKit clicks still activate it.
        harness.process_text_event(TextEvent::key_up(Key::Named(NamedKey::Enter)));
        assert_eq!(
            harness.pop_action(),
            Some((ButtonPress { button: None }, third))
        );
        harness.mouse_click_on(first, Some(PointerButton::Primary));
        assert_eq!(
            harness.pop_action(),
            Some((
                ButtonPress {
                    button: Some(PointerButton::Primary)
                },
                first
            ))
        );
        assert_eq!(harness.focused_widget_id(), Some(first));
        // `accessibility_click_on` doesn't flush actions, so send the request directly.
        harness.process_access_event(accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: third.to_raw().into(),
            data: None,
        });
        assert_eq!(
            harness.pop_action(),
            Some((ButtonPress { button: None }, third))
        );

        // Rove: make the new tab a Tab stop, focus it, then drop the old one.
        harness.edit_widget(tags[2], |mut button| {
            Button::set_focusable(&mut button, true);
        });
        harness.focus_on(Some(third));
        harness.edit_widget(tags[1], |mut button| {
            Button::set_focusable(&mut button, false);
        });
        assert_eq!(harness.focused_widget_id(), Some(third));

        harness.focus_on(Some(after));
        harness.press_tab_key(true);
        assert_eq!(harness.focused_widget_id(), Some(third));
        // The other tabs are skipped, so Shift+Tab wraps around to the last Tab stop.
        harness.press_tab_key(true);
        assert_eq!(harness.focused_widget_id(), Some(after));
        assert_ne!(harness.focused_widget_id(), Some(selected));
    }

    #[test]
    fn edit_button() {
        let image_1 = {
            let label = Label::new("The quick brown fox jumps over the lazy dog")
                .with_style(StyleProperty::FontSize(20.0));
            let label = NewWidget::new(label)
                .with_props(PropertySet::new().with(ContentColor::new(ACCENT_COLOR)));

            let button = NewWidget::new(Button::new(label));

            let mut harness = TestHarness::create_with_size(test_property_set(), button, (50, 50));

            harness.render()
        };

        let image_2 = {
            let button = NewWidget::new(Button::with_text("Hello world"));

            let mut harness = TestHarness::create_with_size(test_property_set(), button, (50, 50));

            harness.edit_root_widget(|mut button| {
                let mut label = Button::child_mut(&mut button);
                let mut label = label.downcast();

                Label::set_text(&mut label, "The quick brown fox jumps over the lazy dog");

                label.insert_prop(ContentColor::new(ACCENT_COLOR));
                Label::insert_style(&mut label, StyleProperty::FontSize(20.0));
            });

            harness.render()
        };

        // We don't use assert_eq because we don't want rich assert
        assert!(image_1 == image_2);
    }

    #[test]
    fn set_properties() {
        let red = crate::palette::css::RED;
        let button = NewWidget::new(Button::with_text("Some random text"));

        let mut harness = TestHarness::create_with_size(test_property_set(), button, (200, 80));

        harness.edit_root_widget(|mut button| {
            button.insert_prop(BorderColor { color: red });
            button.insert_prop(BorderWidth { width: 5.px() });
            button.insert_prop(CornerRadius { radius: 20.px() });
            button.insert_prop(Padding::from_vh(3.px(), 8.px()));

            let mut label = Button::child_mut(&mut button);
            label.insert_prop(ContentColor::new(red));
        });

        assert_render_snapshot!(harness, "button_set_properties");
    }

    #[test]
    fn with_shadows() {
        use crate::palette::css::ORANGE;

        let grid = Grid::with_dimensions(2, 2)
            .with(Button::with_text("A").prepare(), GridParams::pos(0, 0))
            .with(Button::with_text("B").prepare(), GridParams::pos(1, 0))
            .with(Button::with_text("C").prepare(), GridParams::pos(0, 1))
            .with(Button::with_text("D").prepare(), GridParams::pos(1, 1));
        let root_widget = NewWidget::new(grid).with_props(
            PropertySet::new()
                .with(Padding::all(20.px()))
                .with(Gap::new(40.px())),
        );

        let test_params = TestHarnessParams::default().with_size((300, 300));
        let mut harness = TestHarness::create_with(test_property_set(), root_widget, test_params);

        harness.edit_root_widget(|mut grid| {
            {
                let mut button = Grid::get_mut(&mut grid, 0);
                let mut button = button.downcast::<Button>();
                button.insert_prop(BoxShadow::new(ORANGE, (10., 10.)));
            }

            {
                let mut button = Grid::get_mut(&mut grid, 1);
                let mut button = button.downcast::<Button>();
                button.insert_prop(BoxShadow::new(ORANGE, (-10., 10.)).blur(5.px()));
            }

            {
                let mut button = Grid::get_mut(&mut grid, 2);
                let mut button = button.downcast::<Button>();
                button.insert_prop(BoxShadow::new(ORANGE, (-10., -10.)).blur(0.px()));
            }

            {
                let mut button = Grid::get_mut(&mut grid, 3);
                let mut button = button.downcast::<Button>();
                button.insert_prop(BoxShadow::new(ORANGE, (0., 0.)).blur(5.px()));
            }
        });

        assert_render_snapshot!(harness, "button_shadows");

        // Check that slightly changing the blur radius makes the screenshot test fail.
        harness.edit_root_widget(|mut grid| {
            // Copy-pasted from second case above.
            {
                let mut button = Grid::get_mut(&mut grid, 1);
                let mut button = button.downcast::<Button>();
                button.insert_prop(BoxShadow::new(ORANGE, (-10., 10.)).blur(2.5.px()));
            }
        });
        assert_failing_render_snapshot!(harness, "button_shadows");
    }

    /// We document that several other non-interactive widgets in Masonry can be correctly used as children in buttons.
    ///
    /// We validate that each of these actually are correctly supported.
    fn validate_noninteractive_child<W: Widget>(child: NewWidget<W>) {
        let child_id = child.id();
        let mut button = Button::new(child).prepare();
        button.properties.insert(Padding::all(10.px()));
        let button_id = button.id();
        let mut harness = TestHarness::create(test_property_set(), button);

        harness.mouse_move_to_unchecked(child_id);
        let button = harness.get_widget_with_id(button_id);
        assert!(
            button.ctx().is_hovered(),
            "The child shouldn't prevent hover."
        );
        harness.mouse_button_press(None);
        let button = harness.get_widget_with_id(button_id);
        assert!(
            button.ctx().is_pointer_capture_target(),
            "A non-interactive child shouldn't prevent pointer capture."
        );
        harness.mouse_button_release(None);
        let (_, event_id) = harness
            .pop_action::<<Button as Widget>::Action>()
            .expect("There should be an action.");
        assert_eq!(
            event_id, button_id,
            "The event should come from the button."
        );
    }

    #[test]
    fn label_child() {
        let child = Label::new("Some text").prepare();
        validate_noninteractive_child(child);
    }

    #[test]
    fn sized_box_child() {
        let child = SizedBox::empty().width(50.px()).height(50.px()).prepare();
        validate_noninteractive_child(child);
    }

    #[test]
    fn flex_child() {
        let child = Flex::row()
            .with_fixed(Label::new("Some text").prepare())
            .prepare();
        validate_noninteractive_child(child);
    }
    // We could imagine more involved tests, e.g. a button with an icon
    // or a with a keyboard shortcut indicator.

    #[test]
    fn textless_button() {
        let tag = WidgetTag::unique();
        let button = Button::with_text("").prepare().with_tag(tag);
        let parent = Flex::row().with(button, 0.).prepare();

        let params =
            TestHarnessParams::size_and_padding((100, 40), TestHarnessParams::ROOT_PADDING);
        let mut harness = TestHarness::create_with(test_property_set(), parent, params);
        let button_id = harness.get_widget(tag).id();

        assert_render_snapshot!(harness, "button_no_text");

        harness.mouse_click_on(button_id, Some(PointerButton::Primary));
        assert_matches!(
            harness.pop_action::<ButtonPress>(),
            Some((ButtonPress { .. }, ..))
        );
    }
}
