use std::ffi::CString;
use std::io;
use std::os::fd::AsRawFd;
use std::ptr;
use std::time::Instant;

use crate::runner_native::path_policy::MeasuredProbe;
use crate::runner_native::platform::child_io;

pub(in crate::runner_native) fn fexecve_capture(
    measured: &MeasuredProbe,
    input: &[u8],
    max_stdout: usize,
    max_stderr: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    fexecve_capture_with_arguments(
        measured,
        &["feathermark-runner-probe"],
        input,
        max_stdout,
        max_stderr,
        deadline,
    )
}

#[cfg(test)]
pub(in crate::runner_native) fn fexecve_capture_for_test(
    measured: &MeasuredProbe,
    arguments: &[&str],
    input: &[u8],
    max_stdout: usize,
    max_stderr: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    fexecve_capture_with_arguments(measured, arguments, input, max_stdout, max_stderr, deadline)
}

fn fexecve_capture_with_arguments(
    measured: &MeasuredProbe,
    arguments: &[&str],
    input: &[u8],
    max_stdout: usize,
    max_stderr: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    if arguments.is_empty() {
        return Err(invalid("fexecve requires argv[0]"));
    }
    let arguments: Vec<CString> = arguments
        .iter()
        .map(|argument| CString::new(*argument).map_err(|_| invalid("probe argument contains NUL")))
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<_> = arguments.iter().map(|value| value.as_ptr()).collect();
    argv.push(ptr::null());
    let environment = probe_environment()?;
    let mut envp: Vec<_> = environment.iter().map(|value| value.as_ptr()).collect();
    envp.push(ptr::null());

    let mut input_pipe = [-1; 2];
    let mut output_pipe = [-1; 2];
    let mut error_pipe = [-1; 2];
    if unsafe { libc::pipe(input_pipe.as_mut_ptr()) } != 0
        || unsafe { libc::pipe(output_pipe.as_mut_ptr()) } != 0
        || unsafe { libc::pipe(error_pipe.as_mut_ptr()) } != 0
    {
        let error = io::Error::last_os_error();
        close_all(&[input_pipe, output_pipe, error_pipe]);
        return Err(error);
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let error = io::Error::last_os_error();
        close_all(&[input_pipe, output_pipe, error_pipe]);
        return Err(error);
    }
    if pid == 0 {
        unsafe {
            if libc::dup2(input_pipe[0], libc::STDIN_FILENO) < 0
                || libc::dup2(output_pipe[1], libc::STDOUT_FILENO) < 0
                || libc::dup2(error_pipe[1], libc::STDERR_FILENO) < 0
            {
                libc::_exit(125);
            }
            close_all(&[input_pipe, output_pipe, error_pipe]);
            libc::fexecve(measured.file().as_raw_fd(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(126);
        }
    }

    unsafe {
        libc::close(input_pipe[0]);
        libc::close(output_pipe[1]);
        libc::close(error_pipe[1]);
    }
    child_io::collect_child(
        pid,
        child_io::ChildPipes {
            stdin: input_pipe[1],
            stdout: output_pipe[0],
            stderr: error_pipe[0],
        },
        input,
        deadline,
        child_io::OutputLimits {
            stdout: max_stdout,
            stderr: max_stderr,
        },
        "fexecve probe",
    )
}

fn probe_environment() -> io::Result<Vec<CString>> {
    let values = ["PATH=/usr/bin:/bin".to_owned()];
    let mut environment = Vec::with_capacity(values.len());
    for value in values {
        environment.push(CString::new(value).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "probe environment contains NUL")
        })?);
    }
    Ok(environment)
}

fn close_all(pipes: &[[i32; 2]]) {
    for fd in pipes.iter().flatten().copied().filter(|fd| *fd >= 0) {
        unsafe {
            libc::close(fd);
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
