// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Finder / Dock / "Open With" document delivery on macOS.
//!
//! Double-click and Dock-drop do not put paths in `argv`. They arrive as a
//! `kAEOpenDocuments` (`aevt`/`odoc`) Apple Event. GPUI's
//! `application:openURLs:` is a backup, but AppKit installs its own `odoc`
//! handler during startup and can refuse or race the cold-launch event.
//!
//! Pattern (measured by flyleaf / similar Cocoa apps):
//! 1. Observe `NSApplicationWillFinishLaunchingNotification`.
//! 2. In that observer, register with `NSAppleEventManager` (not raw Carbon
//!    `AEInstallEventHandler`, which can steal the event and leave paths
//!    unparsed while also blocking `application:openURLs:`).
//! 3. Coerce each list item to `typeFileURL` and decode via `NSURL`.

use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, sel, AnyThread};
use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString,
    NSURL,
};

/// Shared queue of paths from Finder / Dock open-document events.
fn queue() -> &'static Mutex<Vec<PathBuf>> {
    static Q: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Vec::new()))
}

type OpenHandler = Box<dyn FnMut(Vec<PathBuf>) + Send>;

fn open_handler_slot() -> &'static Mutex<Option<OpenHandler>> {
    static H: OnceLock<Mutex<Option<OpenHandler>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const FILE_URL: u32 = u32::from_be_bytes(*b"furl");

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; this type has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "FieldAssistOpenDocuments"]
    struct Handler;

    impl Handler {
        // SAFETY: signature matches what NSAppleEventManager invokes for odoc.
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            let paths = paths_from_event(event);
            log_open(&format!(
                "handleOpenDocuments: extracted {} path(s)",
                paths.len()
            ));
            deliver(paths);
        }
    }

    unsafe impl NSObjectProtocol for Handler {}
);

/// Call once before `Application::run`. Observes will-finish-launching so the
/// Apple Event handler is installed after AppKit's default and before the
/// cold-launch open-documents event is dispatched.
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let block = RcBlock::new(|_notification: NonNull<NSNotification>| {
            register_handler();
        });
        // SAFETY: AppKit constant name; nil object/queue → main-thread post.
        unsafe {
            let observer = NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationWillFinishLaunchingNotification),
                    None,
                    None,
                    &block,
                );
            // Keep the observer for process lifetime.
            std::mem::forget(observer);
        }
    });
}

/// Register a callback for paths that arrive after the GPUI app is alive
/// (Dock drop while running).
pub fn set_open_handler(handler: impl FnMut(Vec<PathBuf>) + Send + 'static) {
    *open_handler_slot().lock().unwrap() = Some(Box::new(handler));
}

/// Drain any queued open-document paths (cold launch before App).
pub fn take_queued_paths() -> Vec<PathBuf> {
    match queue().lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => Vec::new(),
    }
}

/// Paths from the Apple Event currently being handled, if it is open-documents.
pub fn paths_from_current_open_documents_event() -> Vec<PathBuf> {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let Some(event) = manager.currentAppleEvent() else {
        return Vec::new();
    };
    if event.eventClass() != CORE_EVENT_CLASS || event.eventID() != OPEN_DOCUMENTS {
        return Vec::new();
    }
    paths_from_event(&event)
}

fn register_handler() {
    log_open("register_handler: installing NSAppleEventManager odoc handler");
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let handler: Retained<Handler> = unsafe { msg_send![Handler::alloc(), init] };
    // SAFETY: handler implements the selector; AE manager does not retain it.
    unsafe {
        let object: &AnyObject = &handler;
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            object,
            sel!(handleOpenDocuments:withReplyEvent:),
            CORE_EVENT_CLASS,
            OPEN_DOCUMENTS,
        );
    }
    std::mem::forget(handler);
}

fn paths_from_event(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let Some(list) = event.paramDescriptorForKeyword(DIRECT_OBJECT) else {
        return Vec::new();
    };
    let count = list.numberOfItems();
    if count <= 0 {
        return path_from_descriptor(&list).into_iter().collect();
    }
    let mut out = Vec::with_capacity(count as usize);
    for index in 1..=count {
        if let Some(item) = list.descriptorAtIndex(index) {
            if let Some(path) = path_from_descriptor(&item) {
                out.push(path);
            }
        }
    }
    out
}

fn path_from_descriptor(item: &NSAppleEventDescriptor) -> Option<PathBuf> {
    // Prefer coerce → typeFileURL → UTF-8 URL bytes → NSURL (flyleaf pattern).
    if let Some(coerced) = item.coerceToDescriptorType(FILE_URL) {
        if let Ok(text) = String::from_utf8(coerced.data().to_vec()) {
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(&text)) {
                if let Some(path) = url.path() {
                    let path = path.to_string();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
            if let Some(path) = crate::app::path_from_open_url(&text) {
                return Some(path);
            }
        }
    }
    if let Some(url) = item.fileURLValue() {
        if let Some(path) = url.path() {
            let path = path.to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    if let Some(string) = item.stringValue() {
        let path = string.to_string();
        if path.starts_with('/') {
            return Some(PathBuf::from(path));
        }
        return crate::app::path_from_open_url(&path);
    }
    None
}

fn deliver(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    log_open(&format!("deliver {} path(s): {:?}", paths.len(), paths));
    if let Ok(mut slot) = open_handler_slot().lock() {
        if let Some(handler) = slot.as_mut() {
            handler(paths);
            return;
        }
    }
    if let Ok(mut q) = queue().lock() {
        q.extend(paths);
    }
}

pub fn log_open(msg: &str) {
    use std::io::Write;
    let line = format!(
        "{} {msg}\n",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/fieldassist-open.log")
        .and_then(|mut f| f.write_all(line.as_bytes()));
}
