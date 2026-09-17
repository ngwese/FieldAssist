// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! macOS-native menu tweaks that GPUI's `MenuItem` API cannot express.

use std::sync::OnceLock;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSBezierPath, NSColor, NSImage, NSMenu};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// Titles of View → waveform representation radio items.
const WAVEFORM_RADIO_TITLES: &[&str] = &["Peaks", "Spectrum", "Peaks + Spectrum"];

/// Menu state-image slot size (AppKit scales `onStateImage` to fill this).
const STATE_SLOT: f64 = 14.0;
/// Visible filled-circle diameter inside the slot (~half a normal checkmark mark).
const DOT_DIAMETER: f64 = 5.0;

struct RadioDotImage(objc2::rc::Retained<NSImage>);

// Created once on the main thread; only read from main-thread menu installs.
unsafe impl Send for RadioDotImage {}
unsafe impl Sync for RadioDotImage {}

/// Apply macOS-only menu fixes after `App::set_menus`.
///
/// AppKit's default `autoenablesItems` re-enables any action that GPUI's
/// `validateMenuItem` reports as available (our global `on_action` handlers),
/// which overrides `MenuItem::disabled`. Submenus are not validated that way,
/// so only they appeared greyed. Turning auto-enable off keeps GPUI's
/// `setEnabled_` from `disabled` as the source of truth.
pub fn apply_after_set_menus() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(main_menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    disable_autoenables(&main_menu);
    if let Some(image) = radio_dot_image() {
        apply_waveform_radio_marks_to(&main_menu, image);
    }
}

fn disable_autoenables(menu: &NSMenu) {
    menu.setAutoenablesItems(false);
    let count = menu.numberOfItems();
    for index in 0..count {
        let Some(item) = menu.itemAtIndex(index) else {
            continue;
        };
        if let Some(submenu) = item.submenu() {
            disable_autoenables(&submenu);
        }
    }
}

fn apply_waveform_radio_marks_to(menu: &NSMenu, image: &NSImage) {
    let count = menu.numberOfItems();
    for index in 0..count {
        let Some(item) = menu.itemAtIndex(index) else {
            continue;
        };
        if let Some(submenu) = item.submenu() {
            apply_waveform_radio_marks_to(&submenu, image);
        }
        let title = item.title().to_string();
        if WAVEFORM_RADIO_TITLES.contains(&title.as_str()) {
            unsafe {
                item.setOnStateImage(Some(image));
            }
        }
    }
}

fn radio_dot_image() -> Option<&'static NSImage> {
    static IMAGE: OnceLock<Option<RadioDotImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| Some(create_radio_dot_image()))
        .as_ref()
        .map(|img| img.0.as_ref())
}

fn create_radio_dot_image() -> RadioDotImage {
    // Draw a small circle inside a full-size state slot. AppKit scales
    // `onStateImage` to the slot, so shrinking the image itself has no effect;
    // transparent padding is what makes the visible mark smaller.
    let slot = NSSize::new(STATE_SLOT, STATE_SLOT);
    let handler = RcBlock::new(|_rect: NSRect| {
        NSColor::blackColor().setFill();
        let origin = (STATE_SLOT - DOT_DIAMETER) * 0.5;
        let oval = NSRect::new(
            NSPoint::new(origin, origin),
            NSSize::new(DOT_DIAMETER, DOT_DIAMETER),
        );
        NSBezierPath::bezierPathWithOvalInRect(oval).fill();
        objc2::runtime::Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(slot, false, &handler);
    image.setTemplate(true);
    RadioDotImage(image)
}
