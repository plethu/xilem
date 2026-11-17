// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use masonry::core::{ArcStr, NewWidget, PropertySet};
use masonry::parley::style::FontWeight;
use masonry::parley::{FontFamily, FontFamilyName, GenericFamily, LineHeight, StyleProperty};
use masonry::peniko::Color;
use masonry::properties::{CaretColor, ContentColor, PlaceholderColor, SelectionColor};
use masonry::widgets::{self, TextAction};

use crate::core::{MessageCtx, MessageResult, Mut, View, ViewMarker};
use crate::view::Prop;
use crate::{InsertNewline, Pod, TextAlign, ViewCtx, WidgetView as _};

// FIXME - A major problem of the current approach (always setting the text_input contents)
// is that if the user forgets to hook up the modify the state's contents in the callback,
// the text_input will always be reset to the initial state. This will be very annoying for the user.

type Callback<State, Action> = Box<dyn Fn(&mut State, String) -> Action + Send + Sync + 'static>;

/// A view which displays editable text.
///
/// The text input's current text currently *must* be stored in your app's state, and so
/// will not work if it isn't managed correctly.
/// If the user has made an input, the updated value is provided by the `on_changed`
/// callback, which is the second parameter to this function (where the first is a
/// clone of the current contents).
/// See the examples for how to use this.
///
/// By default, the text input is single-line - that is, pressing Enter <kbd>↵</kbd> does
/// not insert a newline.
/// This can be configured using [`insert_newline`](TextInput::insert_newline) on the
/// returned view.
/// In the default state, Enter <kbd>↵</kbd> being pressed can therefore be used as a
/// "submit" operation.
/// This can be detected by setting the [`on_enter`](TextInput::on_enter) callback.
///
/// # Examples
///
/// Create a basic text input with its content stored in the app state:
///
/// ```
/// # use xilem_masonry as xilem;
/// # use xilem::view::text_input;
/// # use xilem::WidgetView;
///
/// struct State {
///     content: String,
/// }
///
/// fn view(state: &mut State) -> impl WidgetView<State> {
///     text_input(state.content.clone(), |state: &mut State, input: String| {
///         state.content = input;
///     })
/// }
/// ```
///
/// Create a multiline `text_input`:
///
/// ```
/// # use xilem_masonry as xilem;
/// use xilem::view::text_input;
/// use xilem::masonry::widgets::InsertNewline;
/// # use xilem::WidgetView;
///
/// # struct State {
/// #    content: String,
/// # }
///
/// # fn view(state: &mut State) -> impl WidgetView<State> {
/// text_input(state.content.clone(), |state: &mut State, input: String| {
///     state.content = input;
/// })
/// .insert_newline(InsertNewline::OnEnter)
/// # }
/// ```
pub fn text_input<F, State, Action>(contents: String, on_changed: F) -> TextInput<State, Action>
where
    F: Fn(&mut State, String) -> Action + Send + Sync + 'static,
    State: 'static,
{
    TextInput {
        contents,
        on_changed: Box::new(on_changed),
        on_enter: None,
        on_cancel: None,
        text_color: None,
        placeholder: ArcStr::default(),
        text_alignment: TextAlign::default(),
        text_size: masonry::theme::TEXT_SIZE_NORMAL,
        weight: FontWeight::NORMAL,
        line_height: masonry::theme::TEXT_LINE_HEIGHT,
        font: FontFamily::Single(FontFamilyName::Generic(GenericFamily::SystemUi)),
        insert_newline: InsertNewline::default(),
        disabled: false,
        // Since we don't support setting the word wrapping, we can default to
        // not clipping
        clip: true,
    }
}

/// The [`View`] created by [`text_input`].
#[must_use = "View values do nothing unless provided to Xilem."]
pub struct TextInput<State: 'static, Action> {
    contents: String,
    on_changed: Callback<State, Action>,
    on_enter: Option<Callback<State, Action>>,
    on_cancel: Option<Callback<State, Action>>,
    text_color: Option<Color>,
    placeholder: ArcStr,
    text_alignment: TextAlign,
    text_size: f32,
    weight: FontWeight,
    line_height: LineHeight,
    font: FontFamily<'static>,
    insert_newline: InsertNewline,
    disabled: bool,
    clip: bool,
    // TODO: add more attributes of `masonry::widgets::TextInput`
}

impl<State: 'static, Action: 'static> TextInput<State, Action> {
    /// Set the text's color.
    ///
    /// This overwrites the default `ContentColor` property for the inner `TextArea` widget.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Set the insertion caret's color.
    ///
    /// This overwrites the default `CaretColor` property for the inner `TextArea` widget.
    pub fn caret_color(self, color: Color) -> Prop<CaretColor, Self, State, Action> {
        self.prop(CaretColor { color })
    }

    /// Set the selection's color.
    ///
    /// This overwrites the default `SelectionColor` property for the inner `TextArea` widget.
    pub fn selection_color(self, color: Color) -> Prop<SelectionColor, Self, State, Action> {
        self.prop(SelectionColor { color })
    }

    /// Set the string which is shown when the input is empty.
    pub fn placeholder(mut self, placeholder_text: impl Into<ArcStr>) -> Self {
        self.placeholder = placeholder_text.into();
        self
    }

    /// Set the [`PlaceholderColor`] property, which sets the color of the text shown when the input is empty.
    pub fn placeholder_color(self, color: Color) -> Prop<PlaceholderColor, Self, State, Action> {
        self.prop(PlaceholderColor::new(color))
    }

    /// Set the [text alignment](https://en.wikipedia.org/wiki/Typographic_alignment) of the text.
    pub fn text_alignment(mut self, text_alignment: TextAlign) -> Self {
        self.text_alignment = text_alignment;
        self
    }

    /// Sets text size.
    #[doc(alias = "font_size")]
    pub fn text_size(mut self, text_size: f32) -> Self {
        self.text_size = text_size;
        self
    }

    /// Sets font weight.
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    /// Sets line height.
    ///
    /// The default is [`TEXT_LINE_HEIGHT`](masonry::theme::TEXT_LINE_HEIGHT).
    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.line_height = line_height;
        self
    }

    /// Set the [font family](FontFamily) this label will use.
    ///
    /// A font family allows for providing fallbacks. If there is no matching font
    /// for a character, a system font will be used (if the system fonts are enabled).
    pub fn font(mut self, font: impl Into<FontFamily<'static>>) -> Self {
        self.font = font.into();
        self
    }

    /// Configures how this text area handles the user pressing Enter <kbd>↵</kbd>.
    ///
    /// See also [`on_enter`](Self::on_enter), which provides a callback for enter
    /// being used for submitting.
    pub fn insert_newline(mut self, insert_newline: InsertNewline) -> Self {
        self.insert_newline = insert_newline;
        self
    }

    /// Set a callback that will be run when the user presses Enter <kbd>↵</kbd> to submit their input.
    ///
    /// Note that if [`insert_newline`](Self::insert_newline) is `InsertNewline::OnEnter`, this
    /// will never be called.
    pub fn on_enter<F>(mut self, on_enter: F) -> Self
    where
        F: Fn(&mut State, String) -> Action + Send + Sync + 'static,
    {
        self.on_enter = Some(Box::new(on_enter));
        self
    }

    /// Set a callback that will be run when the user presses Escape <kbd>Esc</kbd>.
    pub fn on_cancel<F>(mut self, on_cancel: F) -> Self
    where
        F: Fn(&mut State, String) -> Action + Send + Sync + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
        self
    }

    /// Set the disabled state of the widget.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set whether the contained text will be clipped to the box if it overflows.
    ///
    /// Please note:
    /// 1) We don't currently support scrolling within a text area, so this can make some content
    ///    unviewable (without the user adding spaces and/or copy/pasting to extract content).
    ///    You should probably set this to false for small text inputs (and probably also lower
    ///    the default padding).
    /// 2) This view currently always uses word wrapping, so if there are any linebreaking
    ///    opportunities in the text, they will be taken.
    ///
    /// The default value is true (i.e. clipping is enabled).
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }
}

impl<State: 'static, Action> ViewMarker for TextInput<State, Action> {}
impl<State: 'static, Action: 'static> View<State, Action, ViewCtx> for TextInput<State, Action> {
    type Element = Pod<widgets::TextInput>;
    type ViewState = ();

    fn build(&self, ctx: &mut ViewCtx, _: &mut State) -> (Self::Element, Self::ViewState) {
        // TODO: Maybe we want a shared TextArea View?
        let text_area = widgets::TextArea::new_editable(&self.contents)
            .with_text_alignment(self.text_alignment)
            .with_insert_newline(self.insert_newline)
            .with_style(StyleProperty::FontSize(self.text_size))
            .with_style(StyleProperty::FontWeight(self.weight))
            .with_style(StyleProperty::LineHeight(self.line_height))
            .with_style(StyleProperty::FontFamily(self.font.clone()));

        // TODO - Replace this with properties on the TextInput view
        // once we implement property inheritance or something like it.
        let mut props = PropertySet::new();
        if let Some(color) = self.text_color {
            props.insert(ContentColor { color });
        }

        let text_input =
            widgets::TextInput::from_text_area(NewWidget::new(text_area).with_props(props))
                .with_text_alignment(self.text_alignment)
                .with_clip(self.clip)
                .with_placeholder_style(StyleProperty::FontWeight(self.weight))
                .with_placeholder_style(StyleProperty::LineHeight(self.line_height))
                .with_placeholder_style(StyleProperty::FontFamily(self.font.clone()))
                .with_placeholder(self.placeholder.clone());

        // Ensure that the actions from the *inner* TextArea get routed correctly.
        let id = text_input.area_pod().id();
        ctx.record_action_source(id);

        let mut pod = ctx.create_pod(text_input);
        pod.new_widget.options.disabled = self.disabled;
        (pod, ())
    }

    fn rebuild(
        &self,
        prev: &Self,
        _: &mut Self::ViewState,
        _ctx: &mut ViewCtx,
        mut element: Mut<'_, Self::Element>,
        _: &mut State,
    ) {
        // TODO - Replace this with properties on the TextInput view
        if self.text_color != prev.text_color {
            if let Some(color) = self.text_color {
                element.insert_prop(ContentColor { color });
            } else {
                element.remove_prop::<ContentColor>();
            }
        }
        if self.placeholder != prev.placeholder {
            widgets::TextInput::set_placeholder(&mut element, self.placeholder.clone());
        }

        if self.disabled != prev.disabled {
            element.ctx.set_disabled(self.disabled);
        }

        if self.clip != prev.clip {
            widgets::TextInput::set_clip(&mut element, self.clip);
        }

        if self.text_alignment != prev.text_alignment {
            widgets::TextInput::set_text_alignment(&mut element, self.text_alignment);
        }

        // The placeholder sets its text in the field's own type; its size is a property.
        {
            let mut placeholder = widgets::TextInput::placeholder_mut(&mut element);
            if prev.weight != self.weight {
                widgets::Label::insert_style(
                    &mut placeholder,
                    StyleProperty::FontWeight(self.weight),
                );
            }
            if prev.line_height != self.line_height {
                widgets::Label::insert_style(
                    &mut placeholder,
                    StyleProperty::LineHeight(self.line_height),
                );
            }
            if prev.font != self.font {
                widgets::Label::insert_style(
                    &mut placeholder,
                    StyleProperty::FontFamily(self.font.clone()),
                );
            }
        }

        let mut text_area = widgets::TextInput::text_mut(&mut element);

        // Unlike the other properties, we don't compare to the previous value;
        // instead, we compare directly to the element's text. This is to handle
        // cases like "Previous data says contents is 'fooba', user presses 'r',
        // now data and contents are both 'foobar' but previous data is 'fooba'"
        // without calling `set_text`.

        // This is probably not the right behaviour, but determining what is the right behaviour is hard
        if text_area.widget.text() != &self.contents {
            widgets::TextArea::reset_text(&mut text_area, &self.contents);
        }

        if prev.text_size != self.text_size {
            widgets::TextArea::insert_style(
                &mut text_area,
                StyleProperty::FontSize(self.text_size),
            );
        }
        if prev.weight != self.weight {
            widgets::TextArea::insert_style(&mut text_area, StyleProperty::FontWeight(self.weight));
        }
        if prev.line_height != self.line_height {
            widgets::TextArea::insert_style(
                &mut text_area,
                StyleProperty::LineHeight(self.line_height),
            );
        }
        if prev.font != self.font {
            widgets::TextArea::insert_style(
                &mut text_area,
                StyleProperty::FontFamily(self.font.clone()),
            );
        }
        if prev.insert_newline != self.insert_newline {
            widgets::TextArea::set_insert_newline(&mut text_area, self.insert_newline);
        }
    }

    fn teardown(
        &self,
        _: &mut Self::ViewState,
        ctx: &mut ViewCtx,
        element: Mut<'_, Self::Element>,
    ) {
        ctx.teardown_action_source(element);
    }

    fn message(
        &self,
        _: &mut Self::ViewState,
        message: &mut MessageCtx,
        _: Mut<'_, Self::Element>,
        app_state: &mut State,
    ) -> MessageResult<Action> {
        debug_assert!(
            message.remaining_path().is_empty(),
            "id path should be empty in TextInput::message"
        );
        match message.take_message::<TextAction>() {
            Some(action) => match *action {
                TextAction::Changed(text) => {
                    MessageResult::Action((self.on_changed)(app_state, text))
                }
                TextAction::Entered(text) if self.on_enter.is_some() => {
                    MessageResult::Action((self.on_enter.as_ref().unwrap())(app_state, text))
                }

                TextAction::Entered(_) => {
                    tracing::error!("Textbox::message: on_enter is not set");
                    MessageResult::Stale
                }
                TextAction::Cancelled if self.on_cancel.is_some() => {
                    let text = self.contents.clone();
                    MessageResult::Action((self.on_cancel.as_ref().unwrap())(app_state, text))
                }

                TextAction::Cancelled => MessageResult::Stale,
            },
            None => {
                tracing::error!(?message, "Wrong message type in TextInput::message");
                MessageResult::Stale
            }
        }
    }
}

// --- MARK: TESTS
#[cfg(test)]
mod tests {
    use masonry::core::{NewWidget, WidgetId};
    use masonry::parley::LineHeight;
    use masonry::testing::TestHarness;
    use masonry::theme::default_property_set;
    use masonry::widgets::{self, Flex};

    use super::{TextInput, text_input};
    use crate::core::View;
    use crate::test_util::view_ctx;

    const TEXT_SIZE: f32 = 10.0;

    fn input(line_height: LineHeight) -> TextInput<(), ()> {
        text_input("one\ntwo".into(), |_: &mut (), _| {})
            .text_size(TEXT_SIZE)
            .line_height(line_height)
    }

    fn text_height(harness: &TestHarness<Flex>, area: WidgetId) -> f64 {
        harness
            .get_widget_with_id(area)
            .ctx()
            .content_box()
            .height()
    }

    #[test]
    fn line_height_applies_on_build_and_rebuild() {
        let mut ctx = view_ctx();
        let first = input(LineHeight::FontSizeRelative(1.45));
        let (pod, ()) = View::<(), (), _>::build(&first, &mut ctx, &mut ());
        let input_id = pod.new_widget.id();
        let area_id = pod.new_widget.widget.area_pod().id();
        let root = NewWidget::new(Flex::column().with_fixed(pod.new_widget));
        let mut harness = TestHarness::create(default_property_set(), root);

        // Two lines, laid out with the requested line height from the first frame.
        let expected = f64::from(2.0 * TEXT_SIZE * 1.45);
        let height = text_height(&harness, area_id);
        assert!((height - expected).abs() < 0.5, "{height} != {expected}");

        let second = input(LineHeight::FontSizeRelative(2.0));
        harness.edit_widget_with_id(input_id, |mut element| {
            View::<(), (), _>::rebuild(
                &second,
                &first,
                &mut (),
                &mut ctx,
                element.downcast::<widgets::TextInput>(),
                &mut (),
            );
        });

        let expected = f64::from(2.0 * TEXT_SIZE * 2.0);
        let height = text_height(&harness, area_id);
        assert!((height - expected).abs() < 0.5, "{height} != {expected}");
    }
}
