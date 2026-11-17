// SPDX-License-Identifier: Apache-2.0

use std::sync::LazyLock;

use crate::core::{ArcStr, Property};

/// An explicit accessible name for a text area, independent of its placeholder, with an
/// optional description and invalid state, such as an error said under the field.
///
/// `TextInput` forwards this property to its inner text area.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextAreaLabel {
    /// The name announced by assistive technology.
    pub label: ArcStr,
    /// A description announced after the name, empty for none.
    pub description: ArcStr,
    /// Whether the text is invalid; the description should say why.
    pub invalid: bool,
}

impl TextAreaLabel {
    /// Creates an accessible name.
    pub fn new(label: impl Into<ArcStr>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// Marks the text invalid while `error` holds a reason, and describes the field by it.
    pub fn with_error(mut self, error: Option<impl Into<ArcStr>>) -> Self {
        if let Some(error) = error {
            self.description = error.into();
            self.invalid = true;
        }
        self
    }
}

impl Property for TextAreaLabel {
    fn static_default() -> &'static Self {
        static DEFAULT: LazyLock<TextAreaLabel> = LazyLock::new(TextAreaLabel::default);
        &DEFAULT
    }
}
