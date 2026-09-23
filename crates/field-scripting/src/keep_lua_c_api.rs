// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Keep Lua C API symbols that native modules need but mlua may never call,
//! so the static linker does not dead-strip them from host binaries.

use std::ffi::c_void;
use std::os::raw::c_int;

type LuaState = c_void;

unsafe extern "C" {
    fn luaL_unref(state: *mut LuaState, table: c_int, reference: c_int);
}

/// Take addresses of Lua C API entry points so they remain in the final link.
///
/// Called once from [`crate::host::ScriptHost::new`].
#[inline(never)]
pub(crate) fn retain_for_native_modules() {
    let symbols: &[*const ()] = &[luaL_unref as *const ()];
    std::hint::black_box(symbols);
}
