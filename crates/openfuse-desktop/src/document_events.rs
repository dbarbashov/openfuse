//! Finder's Open Documents Apple event, separate from the windowing delegate.
//!
//! Only queues paths. Admission, file reads and dirty-document confirmation belong
//! to the editor. Installing this bridge does not register file associations.

use std::path::PathBuf;

/// Install before `run_native` so launch-time events are not lost.
#[cfg(target_os = "macos")]
pub fn install() -> Result<(), String> {
    macos::install()
}

#[cfg(not(target_os = "macos"))]
pub fn install() -> Result<(), String> {
    Ok(())
}

/// Pop one event only when the editor is ready to admit it. A modal must leave
/// this queue untouched, so another Open With request cannot replace a prompt.
#[cfg(target_os = "macos")]
pub fn take() -> Option<Result<PathBuf, String>> {
    macos::take()
}

#[cfg(not(target_os = "macos"))]
pub fn take() -> Option<Result<PathBuf, String>> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PathBuf;
    use std::cell::OnceCell;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use objc2::rc::Retained;
    use objc2::{MainThreadOnly, define_class, msg_send, sel};
    use objc2_foundation::{
        MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification,
        NSNotificationCenter, NSObject, NSObjectProtocol, NSString,
    };

    const CORE_EVENT: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    const LIST: u32 = u32::from_be_bytes(*b"list");
    const ERROR_NUMBER: u32 = u32::from_be_bytes(*b"errn");
    const MAX_PENDING: usize = 32;

    #[derive(Default)]
    struct Queue {
        items: VecDeque<Result<PathBuf, String>>,
        overflow: bool,
    }

    impl Queue {
        fn push(&mut self, item: Result<PathBuf, String>) {
            if self.items.len() < MAX_PENDING {
                self.items.push_back(item);
            } else {
                self.overflow = true;
            }
        }

        fn pop(&mut self) -> Option<Result<PathBuf, String>> {
            self.items.pop_front().or_else(|| {
                std::mem::take(&mut self.overflow).then(|| {
                    Err("Too many pending Open With requests; some files were not queued. Open them again after closing the current dialog.".into())
                })
            })
        }
    }

    static QUEUE: Mutex<Queue> = Mutex::new(Queue {
        items: VecDeque::new(),
        overflow: false,
    });

    thread_local! {
        // Keep the Objective-C receiver alive until the application's main
        // thread exits, including before and after the winit event loop starts.
        static HANDLER: OnceCell<Retained<DocumentHandler>> = const { OnceCell::new() };
    }

    define_class!(
        // SAFETY: NSObject has no subclass requirements. The receiver is created
        // and registered on the main thread and does not implement Drop.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct DocumentHandler;

        unsafe impl NSObjectProtocol for DocumentHandler {}

        impl DocumentHandler {
            // NSApplication registers its own handlers during initialization.
            // Reinstall at will-finish-launching, before launch-time documents
            // are dispatched. Observing avoids replacing winit's app delegate.
            #[unsafe(method(openfuseWillFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &NSNotification) {
                register(self);
            }

            // SAFETY: NSAppleEventManager invokes this two-descriptor, void
            // selector on its registered event receiver on the main thread.
            #[unsafe(method(openfuseOpenDocuments:withReplyEvent:))]
            fn open_documents(&self, event: &NSAppleEventDescriptor, reply: &NSAppleEventDescriptor) {
                let items = decode(event);
                let failed = items.iter().any(Result::is_err);
                let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
                let overflow = queue.items.len() + items.len() > MAX_PENDING;
                for item in items {
                    queue.push(item);
                }
                // -1700: errAECoercionFail. -108: memFullErr (bounded queue).
                // The editor also surfaces these failures; no invalid path is
                // interpreted as a URL or passed through a shell.
                if failed || overflow {
                    let code = if overflow { -108 } else { -1700 };
                    reply.setParamDescriptor_forKeyword(
                        &NSAppleEventDescriptor::descriptorWithInt32(code),
                        ERROR_NUMBER,
                    );
                }
            }
        }
    );

    pub(super) fn install() -> Result<(), String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "Finder document events require the main thread".to_string())?;
        HANDLER.with(|slot| {
            slot.get_or_init(|| {
                // SAFETY: inherited NSObject init has this signature.
                let handler: Retained<DocumentHandler> =
                    unsafe { msg_send![DocumentHandler::alloc(mtm), init] };
                // SAFETY: selector takes one NSNotification, and the receiver
                // remains retained in main-thread storage for app lifetime.
                unsafe {
                    NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                        &handler,
                        sel!(openfuseWillFinishLaunching:),
                        Some(&NSString::from_str(
                            "NSApplicationWillFinishLaunchingNotification",
                        )),
                        None,
                    );
                }
                register(&handler);
                handler
            });
        });
        Ok(())
    }

    fn register(handler: &DocumentHandler) {
        // SAFETY: selector signature is defined above. Both install and the
        // launch notification execute on the retained receiver's main thread.
        unsafe {
            NSAppleEventManager::sharedAppleEventManager()
                .setEventHandler_andSelector_forEventClass_andEventID(
                    handler,
                    sel!(openfuseOpenDocuments:withReplyEvent:),
                    CORE_EVENT,
                    OPEN_DOCUMENTS,
                );
        }
    }

    pub(super) fn take() -> Option<Result<PathBuf, String>> {
        QUEUE.lock().unwrap_or_else(|e| e.into_inner()).pop()
    }

    fn decode(event: &NSAppleEventDescriptor) -> Vec<Result<PathBuf, String>> {
        let Some(list) = event.paramDescriptorForKeyword(DIRECT_OBJECT) else {
            return vec![Err("Open With request has no file list".into())];
        };
        let count = list.numberOfItems();
        if list.descriptorType() != LIST || count != 1 {
            return vec![Err(
                "Open With supports one preset at a time; select a single file".into(),
            )];
        }
        (1..=count)
            .map(|index| {
                let url = list
                    .descriptorAtIndex(index)
                    .and_then(|item| item.fileURLValue())
                    .ok_or_else(|| "Open With item is not a file URL".to_string())?;
                if !url.isFileURL() {
                    return Err("Open With supports local file URLs only".into());
                }
                if url.host().is_some_and(|host| {
                    let host = host.to_string();
                    !host.is_empty() && !host.eq_ignore_ascii_case("localhost")
                }) {
                    return Err("Open With file URL must refer to this computer".into());
                }
                let path = url
                    .to_file_path()
                    .ok_or_else(|| "Open With file URL cannot be decoded".to_string())?;
                if !path.is_absolute() {
                    return Err("Open With file path must be absolute".into());
                }
                Ok(path)
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_foundation::{NSString, NSURL};

        fn event_with_files(paths: &[&str]) -> Retained<NSAppleEventDescriptor> {
            let event = NSAppleEventDescriptor::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
                CORE_EVENT, OPEN_DOCUMENTS, None, -1, 0,
            );
            let list = NSAppleEventDescriptor::listDescriptor();
            for path in paths {
                let url = NSURL::from_file_path(path).unwrap();
                list.insertDescriptor_atIndex(
                    &NSAppleEventDescriptor::descriptorWithFileURL(&url),
                    0,
                );
            }
            event.setParamDescriptor_forKeyword(&list, DIRECT_OBJECT);
            event
        }

        #[test]
        fn native_file_url_roundtrip_keeps_unicode_spaces_and_literal_percent() {
            let path = "/tmp/\u{41f}\u{440}\u{435}\u{441}\u{435}\u{442} 100% #1.FUSE";
            assert_eq!(
                decode(&event_with_files(&[path])),
                vec![Ok(PathBuf::from(path))]
            );
        }

        #[test]
        fn malformed_event_never_becomes_an_empty_filename() {
            let event = NSAppleEventDescriptor::nullDescriptor();
            assert!(decode(&event)[0].is_err());
            let event = event_with_files(&[]);
            assert!(decode(&event)[0].is_err());
            event.setParamDescriptor_forKeyword(
                &NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(
                    "https://example.invalid/a.fuse",
                )),
                DIRECT_OBJECT,
            );
            assert!(decode(&event)[0].is_err());
        }

        #[test]
        fn multiple_files_are_rejected_before_conversion() {
            let event = event_with_files(&["/tmp/preset.fuse", "/tmp/another.fuse"]);
            let decoded = decode(&event);
            assert_eq!(decoded.len(), 1);
            assert!(decoded[0].is_err());
        }

        #[test]
        fn bounded_queue_preserves_existing_order_and_reports_overflow_once() {
            let mut queue = Queue::default();
            for n in 0..MAX_PENDING + 7 {
                queue.push(Ok(PathBuf::from(n.to_string())));
            }
            assert_eq!(queue.items.len(), MAX_PENDING);
            for n in 0..MAX_PENDING {
                assert_eq!(queue.pop(), Some(Ok(PathBuf::from(n.to_string()))));
            }
            assert!(queue.pop().unwrap().is_err());
            assert!(queue.pop().is_none());
        }
    }
}
