// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Content ink preserved when the host mutes chrome `theme.foreground`.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Global, Hsla};

/// Bright content foreground saved before chrome remaps `theme.foreground`.
///
/// FieldAssist mutes `Theme::foreground` so dock tabs and menu chrome inherit a
/// quieter color. Widgets that need real content ink should read this global
/// (via [`content_foreground`]) instead of `cx.theme().foreground`.
pub struct ContentForeground(pub Hsla);

impl Global for ContentForeground {}

/// Resolve content ink, falling back to `theme.foreground` when unset.
pub fn content_foreground(cx: &App) -> Hsla {
    cx.try_global::<ContentForeground>()
        .map(|color| color.0)
        .unwrap_or_else(|| cx.theme().foreground)
}
