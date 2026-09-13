# field-audio-playback agent guide

## Realtime quality gates

Code that runs on the **CPAL output callback** (`PlaybackShared::fill_output`
and anything it calls synchronously) must satisfy:

1. **Zero heap allocation** — no `Vec`/`Box`/`String` growth, no `to_vec`, no
   format strings, no trait-object creation.
2. **Zero blocking lock contention** — no `Mutex::lock`, no waiting
   `RwLock::read`/`write`. Atomic loads/stores and `arc_swap` pointer loads are
   allowed. `try_lock` that fails open (passthrough / silence) is allowed;
   prefer never waiting on a mutex from the callback.
3. **No I/O or decode** — no filesystem, no Symphonia, no pager misses that
   decode on this thread.
4. **Bounded work** — pop the prefetch ring, run monitor DSP into a
   preallocated scratch, update atomics / stats.

`dasp::ring_buffer::{Fixed, Bounded}` are **not** suitable for the
prefetch↔callback boundary: they require `&mut self` and would need a mutex.
Use [`PrefetchRing`](src/prefetch.rs) (atomic SPSC) instead.

## Where work belongs

| Work | Thread |
| --- | --- |
| `PlaybackDataProvider::read_interleaved` | prefetch |
| SRC gather into device-rate source frames | prefetch |
| Ring `push_interleaved` (pre-monitor source) | prefetch |
| Ring `pop_interleaved` + silence underruns | **callback** |
| Monitor `process_gathered` / meters | **callback** |
| Monitor silence flush (meter decay after stop) | **callback** |
| Transport / position atomics | either (atomics only) |

Live monitor parameters apply on the callback so audible response tracks the
device buffer (~1 period), not prefetch ring depth (issue #11).

## Prefetch may allocate and lock

The `fa-prefetch` thread owns composition/pager locks, scratch `Vec`s, and
decode. Keep that isolation when adding features. Monitor graph rebuilds
(chain / sample-rate changes) also stay off the callback and publish a new
RT slot via `ArcSwap`.
