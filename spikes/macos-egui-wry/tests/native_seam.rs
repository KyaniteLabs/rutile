use std::sync::{Arc, Mutex};

use macos_egui_wry_spike::{AppKitMainThread, CANDIDATE, MacShell};

#[derive(Clone)]
struct DropSpy {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for DropSpy {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(self.name);
    }
}

#[test]
fn candidate_identity_matches_the_closed_matrix() {
    assert_eq!(CANDIDATE.package, "macos-egui-wry-spike");
    assert_eq!(CANDIDATE.bin, "macos-egui-wry-spike");
    assert_eq!(CANDIDATE.shell_feature, "macos-egui");
}

#[cfg(target_os = "macos")]
#[test]
fn appkit_guard_rejects_a_worker() {
    assert!(
        std::thread::spawn(AppKitMainThread::claim)
            .join()
            .unwrap()
            .is_err()
    );
}

#[test]
fn teardown_drops_webview_before_egui_window() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut shell = MacShell::new(DropSpy {
        name: "window",
        log: Arc::clone(&log),
    });
    shell
        .attach_webview(DropSpy {
            name: "webview",
            log: Arc::clone(&log),
        })
        .unwrap();

    drop(shell);

    assert_eq!(*log.lock().unwrap(), ["webview", "window"]);
}
