use std::sync::Arc;

use iced_widget::Space;
use iced_winit::program::Program;
use iced_winit::program::core;
use iced_winit::program::runtime::Task;
use iced_winit::program::runtime::UserInterface;
use iced_winit::program::runtime::user_interface;
use iced_winit::winit::window::Window;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IcedProgramEvidence {
    pub booted: bool,
    pub updated: bool,
    pub widget_built: bool,
    pub owns_native_window: bool,
}

#[derive(Default)]
struct SeamProgram;

#[derive(Default)]
struct SeamState {
    evidence: IcedProgramEvidence,
    native_window: Option<Arc<Window>>,
}

#[derive(Clone, Debug)]
enum SeamMessage {
    Probe,
    NativeWindowReady(Arc<Window>),
    Resized,
    Focused,
    IpcAccepted,
}

impl Program for SeamProgram {
    type State = SeamState;
    type Message = SeamMessage;
    type Theme = core::theme::Theme;
    type Renderer = iced_renderer::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "FeatherMark iced Wry seam"
    }

    fn settings(&self) -> core::Settings {
        core::Settings::default()
    }

    fn window(&self) -> Option<core::window::Settings> {
        Some(core::window::Settings::default())
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            SeamState {
                evidence: IcedProgramEvidence {
                    booted: true,
                    ..IcedProgramEvidence::default()
                },
                native_window: None,
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        state.evidence.updated = true;
        if let SeamMessage::NativeWindowReady(window) = message {
            state.native_window = Some(window);
            state.evidence.owns_native_window = true;
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: core::window::Id,
    ) -> core::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        let _ = state.native_window.as_ref();
        Space::new().into()
    }
}

pub fn iced_program_lifecycle_probe() -> IcedProgramEvidence {
    let program = SeamProgram;
    let (mut state, _boot_task) = program.boot();
    let _update_task = program.update(&mut state, SeamMessage::Probe);
    build_widget(&program, &state);
    state.evidence.widget_built = true;
    state.evidence
}

pub(crate) struct IcedProgramHost {
    program: SeamProgram,
    state: SeamState,
}

impl IcedProgramHost {
    pub(crate) fn new() -> Self {
        let program = SeamProgram;
        let (state, _task) = program.boot();
        Self { program, state }
    }

    pub(crate) fn attach_window(&mut self, window: Arc<Window>) {
        let _ = self
            .program
            .update(&mut self.state, SeamMessage::NativeWindowReady(window));
        build_widget(&self.program, &self.state);
        self.state.evidence.widget_built = true;
    }

    pub(crate) fn resized(&mut self) {
        let _ = self.program.update(&mut self.state, SeamMessage::Resized);
    }

    pub(crate) fn focused(&mut self) {
        let _ = self.program.update(&mut self.state, SeamMessage::Focused);
    }

    pub(crate) fn ipc_accepted(&mut self) {
        let _ = self
            .program
            .update(&mut self.state, SeamMessage::IpcAccepted);
    }

    pub(crate) fn evidence(&self) -> IcedProgramEvidence {
        self.state.evidence
    }
}

fn build_widget(program: &SeamProgram, state: &SeamState) {
    let mut renderer = iced_renderer::Renderer::new(core::Font::default(), core::Pixels(16.0));
    let view = program.view(state, core::window::Id::unique());
    let user_interface = UserInterface::build(
        view,
        core::Size::new(640.0, 480.0),
        user_interface::Cache::new(),
        &mut renderer,
    );
    drop(user_interface);
}
