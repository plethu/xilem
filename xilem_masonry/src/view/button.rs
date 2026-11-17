// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::any::type_name;
use std::marker::PhantomData;

use masonry::accesskit::{Role, Toggled};
use masonry::core::ArcStr;
pub use masonry::core::PointerButton;
use masonry::widgets::{self, ButtonAccessState, ButtonPress};

use crate::core::{MessageCtx, MessageResult, Mut, View, ViewId, ViewMarker, ViewPathTracker};
use crate::view::{Label, label};
use crate::{Pod, ViewCtx, WidgetView};

/// A button which calls `callback` when the primary mouse button (normally left) is pressed.
///
/// `child` will be the button's contents. and should be a non-interactive widget, such as a
/// [`label`](label::label), or a layout widget containing several non-interactive widgets.
/// This avoids cases where an inner interactive widget "steals" mouse focus from the outer
/// widget, or is inadvertently impossible to interactive with.
///
/// For making a button with default text styling directly from a string, you can
/// use [`text_button`] as a shorthand for `button(label(text), callback)`.
///
/// The button can also be activated using the keyboard when it has the keyboard focus.
/// Currently this happens when <kbd>Space</kbd> or <kbd>↵ Enter</kbd> are pressed, and is not configurable.
/// If you need to handle middle and right clicks on the button, as well as separate handling for
/// touch, you can use [`button_any_pointer`].
///
/// # Examples
///
/// To create a simple button with styled text:
///
/// ```
/// # use xilem_masonry as xilem;
/// use xilem::masonry::parley::style::FontWeight;
/// use xilem::view::{button, label};
/// # use xilem::WidgetView;
///
/// struct State {
///     count: i32,
/// }
///
/// # fn view() -> impl WidgetView<State> {
/// let label = label("Increase").weight(FontWeight::BOLD);
///
/// button(label, |state: &mut State| {
///     state.count += 1;
/// })
/// # }
/// ```
///
/// To create a button with more complex (non-interactive) contents children:
///
/// ```
/// # use xilem_masonry as xilem;
/// use xilem::masonry::parley::style::FontWeight;
/// use xilem::view::{button, label, flex_row, FlexExt};
/// # use xilem::WidgetView;
/// # type State = u32;
///
/// # fn view() -> impl WidgetView<State> {
/// let children = flex_row((
///     label("👍").flex(1.0),
///     label("Like").weight(FontWeight::BOLD),
/// ));
///
/// button(children, |_: &mut State| {})
/// # }
/// ```
pub fn button<
    State: 'static,
    Action,
    V: WidgetView<State, Action>,
    F: Fn(&mut State) -> Action + Send + 'static,
>(
    child: V,
    callback: F,
) -> Button<
    State,
    Action,
    impl Fn(&mut State, Option<PointerButton>) -> MessageResult<Action> + Send + 'static,
    V,
> {
    Button {
        child,
        callback: move |state: &mut State, button| match button {
            None | Some(PointerButton::Primary) => MessageResult::Action(callback(state)),
            _ => MessageResult::Nop,
        },
        disabled: false,
        access_state: ButtonAccessState::default(),
        role: Role::Button,
        accessible_label: None,
        accessible_description: None,
        focusable: true,
        phantom: PhantomData,
    }
}

/// A button with default styled text.
///
/// This is equivalent to `button(label(text), callback)`, and is useful for
/// making buttons quickly from string literals.
/// For more advanced text styling, prefer [`button`].
pub fn text_button<State: 'static, Action>(
    text: impl Into<ArcStr>,
    callback: impl Fn(&mut State) -> Action + Send + Sync + 'static,
) -> Button<
    State,
    Action,
    impl Fn(&mut State, Option<PointerButton>) -> MessageResult<Action> + Send + Sync + 'static,
    Label,
> {
    button(label(text), callback)
}

/// A button which calls `callback` when pressed with any mouse button, providing
/// the specific mouse button.
///
/// Note that the callback may be called with `None` as the pointer, which indicates
/// that the button was activated with the keyboard or a touch screen (see also [`ButtonPress`]).
/// There is not currently any support for detecting when <kbd>≣ Menu</kbd> was pressed
/// (so as to treat that as a right click, for example).
/// Similarly, there is not currently long-press support.
///
/// For more documentation and examples, see [`button`].
pub fn button_any_pointer<State: 'static, Action, V: WidgetView<State, Action>>(
    child: V,
    callback: impl Fn(&mut State, Option<PointerButton>) -> Action + Send + Sync + 'static,
) -> Button<
    State,
    Action,
    impl Fn(&mut State, Option<PointerButton>) -> MessageResult<Action> + Send + Sync + 'static,
    V,
> {
    Button {
        child,
        callback: move |state: &mut State, button| MessageResult::Action(callback(state, button)),
        disabled: false,
        access_state: ButtonAccessState::default(),
        role: Role::Button,
        accessible_label: None,
        accessible_description: None,
        focusable: true,
        phantom: PhantomData,
    }
}

/// The [`View`] created by [`button`].
///
/// See `button`'s documentation for more context.
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct Button<State, Action, F, V> {
    child: V,
    callback: F,
    disabled: bool,
    access_state: ButtonAccessState,
    role: Role,
    accessible_label: Option<ArcStr>,
    accessible_description: Option<ArcStr>,
    focusable: bool,
    phantom: PhantomData<fn(State) -> Action>,
}

impl<State, Action, F, V> Button<State, Action, F, V> {
    /// Set the disabled state of the widget.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Reports whether the content this button controls is expanded or collapsed.
    ///
    /// Screen readers announce this state, so a disclosure button's name doesn't need arrows.
    /// Without this call the button reports no expanded state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.access_state.expanded = Some(expanded);
        self
    }

    /// Reports whether this button is the selected item, for example in a picker, list or tab row.
    ///
    /// Without this call the button reports no selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.access_state.selected = Some(selected);
        self
    }

    /// Reports this button as a toggle button that is pressed or not.
    ///
    /// Takes a `bool` or a [`Toggled`], whose `Mixed` value reports a partly pressed state.
    /// Without this call the button is a plain button.
    pub fn toggled(mut self, toggled: impl Into<Toggled>) -> Self {
        self.access_state.toggled = Some(toggled.into());
        self
    }

    /// Sets the accessibility role the button reports, [`Role::Button`] by default.
    ///
    /// Use [`Role::Tab`] with [`selected`](Self::selected) for a tab in a [`Role::TabList`].
    /// AccessKit doesn't name tabs from their contents, so also set an
    /// [`accessible_label`](Self::accessible_label).
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the name assistive technology announces, replacing the name taken from the contents.
    pub fn accessible_label(mut self, label: impl Into<ArcStr>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    /// Sets the description assistive technology reads after the name, such as why the button
    /// is unavailable.
    pub fn accessible_description(mut self, description: impl Into<ArcStr>) -> Self {
        self.accessible_description = Some(description.into());
        self
    }

    /// Sets whether Tab and Shift+Tab can focus the button. The default is `true`.
    ///
    /// A button that isn't focusable can still be focused by pointer presses, by code and by
    /// assistive technology, and still activates, like an HTML `tabindex="-1"` element.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }
}

/// Use a distinctive number here, to be able to catch bugs.
/// In case the generational-id view path in `View::Message` leads to the wrong view.
/// This is a randomly generated 32 bit number - 3502359742 in decimal.
const BUTTON_CONTENT_VIEW_ID: ViewId = ViewId::new(0xd0c1c4be);

impl<State, Action, F, V> ViewMarker for Button<State, Action, F, V> {}
impl<F, V, State, Action> View<State, Action, ViewCtx> for Button<State, Action, F, V>
where
    State: 'static,
    Action: 'static,
    V: WidgetView<State, Action>,
    F: Fn(&mut State, Option<PointerButton>) -> MessageResult<Action> + Send + Sync + 'static,
{
    type Element = Pod<widgets::Button>;
    type ViewState = V::ViewState;

    fn build(&self, ctx: &mut ViewCtx, app_state: &mut State) -> (Self::Element, Self::ViewState) {
        let (child, child_state) = ctx.with_id(BUTTON_CONTENT_VIEW_ID, |ctx| {
            View::<State, Action, _>::build(&self.child, ctx, app_state)
        });
        (
            ctx.with_action_widget(|ctx| {
                let mut button = widgets::Button::new(child.new_widget)
                    .with_access_state(self.access_state)
                    .with_role(self.role)
                    .with_focusable(self.focusable);
                if let Some(label) = &self.accessible_label {
                    button = button.with_accessible_label(label.clone());
                }
                if let Some(description) = &self.accessible_description {
                    button = button.with_accessible_description(description.clone());
                }
                let mut pod = ctx.create_pod(button);
                pod.new_widget.options.disabled = self.disabled;
                pod
            }),
            child_state,
        )
    }

    fn rebuild(
        &self,
        prev: &Self,
        state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) {
        if prev.disabled != self.disabled {
            element.ctx.set_disabled(self.disabled);
        }
        if prev.access_state != self.access_state {
            widgets::Button::set_access_state(&mut element, self.access_state);
        }
        if prev.role != self.role {
            widgets::Button::set_role(&mut element, self.role);
        }
        if prev.accessible_label != self.accessible_label {
            widgets::Button::set_accessible_label(&mut element, self.accessible_label.clone());
        }
        if prev.accessible_description != self.accessible_description {
            widgets::Button::set_accessible_description(
                &mut element,
                self.accessible_description.clone(),
            );
        }
        if prev.focusable != self.focusable {
            widgets::Button::set_focusable(&mut element, self.focusable);
        }
        ctx.with_id(BUTTON_CONTENT_VIEW_ID, |ctx| {
            View::<State, Action, _>::rebuild(
                &self.child,
                &prev.child,
                state,
                ctx,
                widgets::Button::child_mut(&mut element).downcast(),
                app_state,
            );
        });
    }

    fn teardown(
        &self,
        view_state: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
    ) {
        ctx.with_id(BUTTON_CONTENT_VIEW_ID, |ctx| {
            View::<State, Action, _>::teardown(
                &self.child,
                view_state,
                ctx,
                widgets::Button::child_mut(&mut element).downcast(),
            );
        });
        ctx.teardown_action_source(element);
    }

    fn message(
        &self,
        view_state: &mut Self::ViewState,
        message: &mut MessageCtx,
        mut element: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        match message.take_first() {
            Some(BUTTON_CONTENT_VIEW_ID) => self.child.message(
                view_state,
                message,
                widgets::Button::child_mut(&mut element).downcast(),
                app_state,
            ),
            None => match message.take_message::<ButtonPress>() {
                Some(press) => (self.callback)(app_state, press.button),
                None => {
                    // TODO: Panic?
                    tracing::error!(
                        "Wrong message type in Button::message: {message:?} expected {}",
                        type_name::<ButtonPress>()
                    );
                    MessageResult::Stale
                }
            },
            _ => {
                tracing::warn!("Got unexpected id path in Button::message");
                MessageResult::Stale
            }
        }
    }
}
