//! Adds only file-open delivery to winit's existing AppKit delegate.
#[cfg(feature = "test-control")]
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSURL};

pub(super) fn install() -> Result<(), String> {
    MainThreadMarker::new().ok_or("File-open delivery must bind on the main thread")?;
    let class = AnyClass::get(c"WinitApplicationDelegate")
        .ok_or("winit application delegate is unavailable")?;
    if class.instance_method(sel!(application:openURLs:)).is_some() {
        return Err("Application delegate already owns file-open delivery".into());
    }
    // SAFETY: Objective-C IMP erases arguments. This implementation has exactly
    // the v@:@@ signature registered below (self, selector, application, URL array).
    let implementation: Imp = unsafe {
        std::mem::transmute::<
            unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSApplication, &NSArray<NSURL>),
            Imp,
        >(receive_urls)
    };
    // SAFETY: The runtime owns this class; installation happens on the main
    // thread before run_app. class_addMethod never replaces an existing method.
    let added = unsafe {
        objc2::ffi::class_addMethod(
            (class as *const AnyClass).cast_mut(),
            sel!(application:openURLs:),
            implementation,
            c"v@:@@".as_ptr(),
        )
    };
    if added.as_bool() {
        Ok(())
    } else {
        Err("Could not install native file-open delivery".into())
    }
}

unsafe extern "C-unwind" fn receive_urls(
    _receiver: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
    urls: &NSArray<NSURL>,
) {
    if urls.len() > crate::document_manager::MAX_OPEN_DOCUMENTS {
        super::open_events::reject_open_delivery("Too many files in one open request");
        return;
    }
    let mut values = Vec::with_capacity(urls.len());
    for url in urls {
        let Some(value) = url.absoluteString() else {
            super::open_events::reject_open_delivery("The file URL is invalid");
            return;
        };
        if value.len() > 8192 {
            super::open_events::reject_open_delivery("The file URL is too long");
            return;
        }
        values.push(value.to_string());
    }
    if let Err(error) = super::open_events::forward_open_urls(values) {
        super::open_events::reject_open_delivery(&error);
    }
}

#[cfg(feature = "test-control")]
pub(super) fn exercise(path: &std::path::Path) -> Result<(), String> {
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new().ok_or("File-open probe must run on the main thread")?;
    let url =
        url::Url::from_file_path(path).map_err(|()| "File-open probe requires an absolute path")?;
    let url =
        NSURL::URLWithString(&NSString::from_str(url.as_str())).ok_or("Probe URL was rejected")?;
    let urls = NSArray::from_retained_slice(&[url]);
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: NSApplication owns its delegate; install verified the selector and
    // its argument ABI. Both the application and URL array remain alive here.
    unsafe {
        let delegate: *mut AnyObject = msg_send![&*app, delegate];
        if delegate.is_null() {
            return Err("Application has no delegate".into());
        }
        let _: () = msg_send![delegate, application: &*app, openURLs: &*urls];
    }
    Ok(())
}
