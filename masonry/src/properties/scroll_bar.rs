// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use crate::core::Property;
use crate::peniko::Color;
use crate::theme;

/// Declares if the scroll bar collapses when not being hovered.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Collapsible(pub bool);

impl Property for Collapsible {
    fn static_default() -> &'static Self {
        static DEFAULT: Collapsible = Collapsible(false);
        &DEFAULT
    }
}

/// The colours of a scroll bar's thumb: its fill and its 1px edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollThumbColor {
    /// The thumb's fill.
    pub fill: Color,
    /// The thumb's edge.
    pub edge: Color,
}

impl Property for ScrollThumbColor {
    fn static_default() -> &'static Self {
        static DEFAULT: ScrollThumbColor = ScrollThumbColor {
            fill: theme::SCROLLBAR_COLOR,
            edge: theme::SCROLLBAR_BORDER_COLOR,
        };
        &DEFAULT
    }
}

impl Default for ScrollThumbColor {
    fn default() -> Self {
        *Self::static_default()
    }
}
