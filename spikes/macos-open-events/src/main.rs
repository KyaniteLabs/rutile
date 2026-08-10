//! Compatibility proof for macOS open-event delivery via winit 0.30.13 user events.
//!
//! Proves: open URL classifier, `EventLoopProxy<MacUserEvent>` wake, and cold-launch
//! CLI open. Does not replace winit's `WinitApplicationDelegate` (that breaks the loop).

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use iced_winit::winit::application::ApplicationHandler;
use iced_winit::winit::event::WindowEvent;
use iced_winit::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use iced_winit::winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use iced_winit::winit::window::WindowId;
use rutile_app::platform::macos::{
    AppKitMainThread, MacOpenRequest, MacUserEvent, ProductSession, bind_open_proxy,
    forward_open_urls,
};

fn main() {
    if let Err(error) = run_proof() {
        eprintln!("macos-open-events-spike failed: {error}");
        std::process::exit(1);
    }
    println!("macos-open-events-spike-ok hook=user-event-proxy winit=0.30.13");
}

struct ProofRunner {
    saw_user_event: bool,
}

impl ApplicationHandler<MacUserEvent> for ProofRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: MacUserEvent) {
        if let MacUserEvent::OpenUrls(urls) = event {
            assert_eq!(urls, vec!["file:///tmp/proof.md".to_owned()]);
            self.saw_user_event = true;
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
    }
}

fn run_proof() -> Result<(), String> {
    AppKitMainThread::claim().map_err(|error| error.to_string())?;

    assert!(matches!(
        ProductSession::classify_open_urls(&[]),
        MacOpenRequest::Malformed { .. }
    ));
    assert!(matches!(
        ProductSession::classify_open_urls(&["a.md".into(), "b.md".into()]),
        MacOpenRequest::UnsupportedMulti { count: 2 }
    ));
    assert!(matches!(
        ProductSession::classify_open_urls(&["file:///tmp/x.md".into()]),
        MacOpenRequest::File(_)
    ));

    let mut event_loop = EventLoop::<MacUserEvent>::with_user_event()
        .build()
        .map_err(|error| error.to_string())?;
    bind_open_proxy(event_loop.create_proxy());

    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = forward_open_urls(vec!["file:///tmp/proof.md".into()]);
        let _ = done_tx.send(result);
    });

    let mut runner = ProofRunner {
        saw_user_event: false,
    };
    event_loop
        .run_app_on_demand(&mut runner)
        .map_err(|error| error.to_string())?;

    done_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "proxy send did not complete".to_string())??;

    if !runner.saw_user_event {
        return Err("user_event handler never received OpenUrls".into());
    }

    let path = PathBuf::from("/tmp/rutile-open-events-proof.md");
    let _ = std::fs::write(&path, "# proof\n");
    let session = ProductSession::open(&path).map_err(|error| error.to_string())?;
    assert_eq!(session.source(), "# proof\n");
    let _ = std::fs::remove_file(path);

    Ok(())
}
