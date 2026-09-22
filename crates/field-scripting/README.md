# field-scripting

Shared Lua 5.4 host for FieldAssist, `field-batch`, and custom embeddings.

The crate exposes a `field.*` namespace (session, composition, media, workflow,
layout, fs, …) and an `app` object, then lets you swap in different *backends*
to adapt the same Lua surface to headless batch work or a full desktop GUI.

---

## Concepts

### `app` vs `field`

| Table | Purpose |
|-------|---------|
| `app` | Identity / environment: `app.name`, `app.args`, `app.alert` |
| `field` | Domain APIs: `field.session`, `field.composition`, `field.media`, … |

### `HeadlessWorld`

[`HeadlessWorld`] is the in-process store used by `field-batch` and tests.
It holds:

* `session: Session` — the active session (open documents, groups, workflow
  name, properties).
* `docs: HashMap<DocumentId, OpenDocument>` — loaded compositions with their
  region collections, markers, and playhead.
* `media_store: Arc<Mutex<MediaStore>>` — shared media pool.

`HeadlessWorld` is a **plain data container** — no Lua, no backend trait.

### `ScriptBackend`

[`ScriptBackend`] is the object-safe trait that the Lua `field.*` bindings call
into.  All session, document, and media operations go through the backend so the
same Lua code can run against different stores.

Key method groups:

| Group | Examples |
|-------|---------|
| Session metadata | `session_id`, `session_path`, `session_workflow`, `session_properties`, groups |
| Session routing | `which: Option<SessionId>` — `None` = focused world; `Some(id)` = detached |
| World-only mutations | `open_path`, `reset_session`, `open_detached_session`, `load_shared_session_file` |
| Document metadata | `display_name`, `document_group`, `document_state`, `document_properties` |
| Document content | `with_open_document`, `with_open_document_mut` (type-erased closures) |
| Channel layout | `apply_document_channel_layout` |
| Media pool | `add_media`, `remove_media`, `list_media`, `get_media` |
| Desktop chrome stubs | `composition_parent`, `composition_children`, `composition_codec`, `composition_monitor_chain`, `composition_playback_channels` — default `Ok(None)` / `Err(unsupported)` |

### `HeadlessBackend`

[`HeadlessBackend`] implements `ScriptBackend` over a `HeadlessWorld`.

```rust
pub struct HeadlessBackend {
    world: Rc<RefCell<HeadlessWorld>>,
    detached_sessions: HashMap<SessionId, Session>,
}
```

* `world` — the focused session/doc/media store.
* `detached_sessions` — sessions loaded via `field.session.open("…")` without
  replacing the focused session.  Each has its own `SessionId` and can be
  addressed by passing `Some(id)` to session methods.

`HeadlessBackend` is the default backend used by `ScriptHost::new`.

### `DesktopBackend` *(FieldAssist)*

FieldAssist provides `DesktopBackend` in its application crate.  It implements
`ScriptBackend` against the live GPUI model, including desktop composition
chrome (`monitor_chain`, `playback_channels`, and layout metadata) and the
FieldAssist media-pool reference checks.  Detached sessions opened through
`field.session.open()` remain independent of the focused desktop session.

---

## Embedding a custom host

### Minimal setup

```rust
use field_scripting::{HostProfile, ScriptHost};

let mut host = ScriptHost::new(HostProfile {
    name: "my-tool",
    config_dir: None,
})?;
let out = host.eval("return field.session.focused().id ~= nil");
assert_eq!(out.result.as_deref(), Some("true"));
```

### Injecting a custom backend

Implement `ScriptBackend` for your own store, then construct the host with it:

```rust
use std::cell::RefCell;
use std::rc::Rc;
use field_scripting::{HostProfile, ScriptBackend, ScriptHost};

struct MyBackend { /* … */ }
impl ScriptBackend for MyBackend {
    // … implement all required methods …
#   fn session_id(&self, _: Option<field_session::SessionId>) -> String { todo!() }
#   // (all other methods)
}

let backend: Rc<RefCell<dyn ScriptBackend>> = Rc::new(RefCell::new(MyBackend { /* … */ }));
let host = ScriptHost::with_backend(
    HostProfile { name: "my-tool", config_dir: None },
    backend,
)?;
```

### Using `HeadlessWorld` directly (tests)

```rust
use field_scripting::{HostProfile, ScriptHost};
use field_scripting::HeadlessWorld;

let world = HeadlessWorld::new();
let mut host = ScriptHost::with_world(
    HostProfile { name: "test", config_dir: None },
    world,
)?;
host.eval("field.session.focused().properties = {env='test'}");
```

---

## Detached sessions

`field.session.open("path/to/file.fasession")` loads a `.fasession` into the
*detached* map and returns a `LuaSession` identified by that session's UUID.
The focused world session is **not** replaced.

```lua
local archive = field.session.open("/archive/2026-06-01.fasession")
print(archive.id)           -- UUID of the archived session
print(archive.properties.project)

local current = field.session.focused()
print(current.id)           -- unchanged world session id
```

In Phase 1, detached sessions expose metadata (id, path, groups, properties,
compositions list) but their documents are not pre-opened into the world.

---

## AGENTS.md compliance

* `cargo fmt` before committing.
* `cargo build` and `cargo test -p field-scripting -p field-batch` must pass
  with **no errors and no warnings**.
* Code on the CPAL audio callback must not allocate or take blocking locks.
