// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host binaries that embed vendored Lua must export the Lua C API so
//! `dlopen`'d modules (e.g. `lsqlite3.so`) can resolve `lua_*` symbols.
//! See mlua FAQ: "Loading a C module fails with undefined symbol: lua_xxx".

fn main() {
    // Propagates to final products (bins/cdylibs) that link this crate.
    #[cfg(unix)]
    {
        println!("cargo:rustc-link-arg=-rdynamic");
    }
}
