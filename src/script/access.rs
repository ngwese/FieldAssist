// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::ptr::NonNull;

use gpui_kit::{Context, Window};

use crate::app::AppView;

#[derive(Clone, Copy)]
struct Entered {
    view: NonNull<AppView>,
    window: NonNull<Window>,
    cx: NonNull<Context<'static, AppView>>,
}

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static ENTERED: Cell<Option<Entered>> = const { Cell::new(None) };
}

pub struct EnterGuard;

impl Drop for EnterGuard {
    fn drop(&mut self) {
        DEPTH.with(|depth| {
            let next = depth.get().saturating_sub(1);
            depth.set(next);
            if next == 0 {
                ENTERED.with(|slot| slot.set(None));
            }
        });
    }
}

/// Borrow the live `AppView` for the duration of a Lua eval.
pub fn enter(view: &mut AppView, window: &mut Window, cx: &mut Context<AppView>) -> EnterGuard {
    DEPTH.with(|depth| {
        let n = depth.get();
        if n == 0 {
            ENTERED.with(|slot| {
                slot.set(Some(Entered {
                    view: NonNull::from(view),
                    window: NonNull::from(window),
                    cx: NonNull::from(unsafe {
                        &mut *(cx as *mut Context<AppView> as *mut Context<'static, AppView>)
                    }),
                }));
            });
        }
        depth.set(n + 1);
    });
    EnterGuard
}

pub fn is_entered() -> bool {
    DEPTH.with(|depth| depth.get() > 0)
}

pub fn with_view<R>(
    f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>) -> R,
) -> Result<R, String> {
    ENTERED.with(|slot| {
        let Some(mut entered) = slot.get() else {
            return Err("script is not running inside the app".into());
        };
        unsafe {
            Ok(f(
                entered.view.as_mut(),
                entered.window.as_mut(),
                entered.cx.as_mut(),
            ))
        }
    })
}

pub fn try_invoke_command(command_id: &str) -> Option<Result<(), String>> {
    if !is_entered() {
        return None;
    }
    Some(
        with_view(|view, window, cx| view.invoke_command(command_id, window, cx))
            .and_then(|result| result),
    )
}
