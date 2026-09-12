# field-audio-playback agent guide

## Realtime quality gates

Code that runs on the **CPAL output callback** (`PlaybackShared::fill_output`
and anything it calls synchronously) must satisfy:

1. **Zero heap allocation** — no `Vec`/`Box`/`String` growth, no `to_vec`, no
   format strings, no trait-object creation.
2. **Zero blocking lock contention** — no `Mutex::lock`, no waiting
   `RwLock::read`/`write`. Atomic loads/stores and `arc_swap` pointer loads are
   allowed. Prefer never touching a mutex from the callback at all.
3. **No I/O or decode** — no filesystem, no Symphonia, no pager misses that
   decode on this thread.
4. **Bounded work** — only copy from the prefetch ring and update atomics /
   stats. Sample-rate conversion, monitor DSP, and provider reads belong on the
   prefetch thread.

`dasp::ring_buffer::{Fixed, Bounded}` are **not** suitable for the
prefetch↔callback boundary: they require `&mut self` and would need a mutex.
Use [`PrefetchRing`](src/prefetch.rs) (atomic SPSC) instead.

## Where work belongs

| Work | Thread |
| --- | --- |
| `PlaybackDataProvider::read_interleaved` | prefetch |
| Monitor `process_gathered` / SRC gather | prefetch |
| Ring `push_interleaved` | prefetch |
| Ring `pop_interleaved` + silence underruns | **callback** |
| Transport / position atomics | either (atomics only) |

## Prefetch may allocate and lock

The `fa-prefetch` thread owns composition/pager locks, scratch `Vec`s, and
decode. Keep that isolation when adding features.
