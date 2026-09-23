# Building lsqlite3 for field-batch

field-batch embeds **Lua 5.4** (mlua vendored). Native C modules must match
that ABI, leave Lua symbols unresolved so they bind to the host, and install
as **`lsqlite3.so`** (even on macOS — field-batch's `package.cpath` does not
search `.dylib`).

This directory builds the **lsqlite3complete** amalgamation (SQLite linked in)
but names the module **`lsqlite3`** so scripts can `require("lsqlite3")`.

## Prerequisites

- C compiler (`cc`)
- `curl`, `unzip`, `tar`
- A field-batch binary rebuilt from a tree that links with `-rdynamic` (Unix)
  and retains Lua C API symbols (including ones mlua may not call, e.g.
  `luaL_unref`) so `dlopen`'d modules can resolve them from the process

Lua 5.4 headers are detected via `pkg-config` / Homebrew `lua@5.4`, or
fetched automatically from lua.org when `LUA_INCDIR` is unset.

## Build

```bash
cd docs/examples/field-batch/lsqlite-module
make
```

Optional override:

```bash
make LUA_INCDIR=/usr/include/lua5.4
```

Produces `./lsqlite3.so`. Do not commit the `.so` or downloaded sources;
`make clean` removes them.

### What the Makefile does

1. Downloads pinned lsqlite3 0.9.6 sources (amalgamation zip from the
   bhhaskin/lsqlite3complete release mirror).
2. Compiles `lsqlite3.c` + `sqlite3.c` into `lsqlite3.so` with
   `luaopen_lsqlite3` (no second `liblua`).
3. On macOS uses `-bundle -undefined dynamic_lookup`; on Linux `-shared -fPIC`.

## Run the demo

From this directory (so `./?.so` finds the module):

```bash
field-batch index-audio-files.lua /path/to/media ./audio-files.sqlite
```

Arguments:

1. Media root directory (default `.`)
2. Output SQLite path (default `audio-files.sqlite`)

The script enables native modules, recursively finds common audio extensions
via `field.fs.find_files`, inserts absolute paths into `audio_files`, and
closes the database.

## Windows

field-batch looks for `lsqlite3.dll` on `package.cpath`. Building that needs
MSVC or MinGW, Lua 5.4 headers, and a DLL that leaves Lua imports unresolved
against the host — not covered by this Makefile.

## See also

- [SCRIPTING.md](../../../SCRIPTING.md) — `field.scripting.enable_native_modules`
- Upstream docs: <http://lua.sqlite.org/>
