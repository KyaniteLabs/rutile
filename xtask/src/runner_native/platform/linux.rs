use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::ptr;

use crate::runner_native::path_policy::MeasuredProbe;
const MAX_CHILD_OUTPUT: usize = 64 * 1024 + 8;

pub(in crate::runner_native) fn fexecve_capture(
    measured: &MeasuredProbe,
    input: &[u8],
) -> io::Result<Vec<u8>> {
    let mut input_pipe = [0; 2];
    let mut output_pipe = [0; 2];
    if unsafe { libc::pipe(input_pipe.as_mut_ptr()) } != 0
        || unsafe { libc::pipe(output_pipe.as_mut_ptr()) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        close_all(&input_pipe, &output_pipe);
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        unsafe {
            libc::close(input_pipe[1]);
            libc::close(output_pipe[0]);
            if libc::dup2(input_pipe[0], libc::STDIN_FILENO) < 0
                || libc::dup2(output_pipe[1], libc::STDOUT_FILENO) < 0
            {
                libc::_exit(125);
            }
            libc::close(input_pipe[0]);
            libc::close(output_pipe[1]);
            let name = CString::new("feathermark-runner-probe").expect("literal has no NUL");
            let environment = probe_environment().unwrap_or_else(|_| libc::_exit(124));
            let argv = [name.as_ptr(), ptr::null()];
            let mut envp: Vec<_> = environment.iter().map(|value| value.as_ptr()).collect();
            envp.push(ptr::null());
            libc::fexecve(measured.file().as_raw_fd(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(126);
        }
    }

    unsafe {
        libc::close(input_pipe[0]);
        libc::close(output_pipe[1]);
    }
    let mut child_stdin = unsafe { std::fs::File::from_raw_fd(input_pipe[1]) };
    child_stdin.write_all(input)?;
    drop(child_stdin);
    let mut child_stdout = unsafe { std::fs::File::from_raw_fd(output_pipe[0]) };
    let mut output = Vec::new();
    std::io::Read::by_ref(&mut child_stdout)
        .take((MAX_CHILD_OUTPUT + 1) as u64)
        .read_to_end(&mut output)?;
    let mut status = 0;
    if unsafe { libc::waitpid(pid, &mut status, 0) } != pid {
        return Err(io::Error::last_os_error());
    }
    if output.len() > MAX_CHILD_OUTPUT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "probe output exceeds fixed bound",
        ));
    }
    if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
        return Err(io::Error::other(format!(
            "fexecve probe exited with wait status {status}"
        )));
    }
    Ok(output)
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

fn close_all(input: &[i32; 2], output: &[i32; 2]) {
    for fd in input.iter().chain(output) {
        unsafe {
            libc::close(*fd);
        }
    }
}
