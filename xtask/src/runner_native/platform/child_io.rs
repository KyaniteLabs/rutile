use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::time::Instant;

pub(in crate::runner_native) struct ChildPipes {
    pub stdin: RawFd,
    pub stdout: RawFd,
    pub stderr: RawFd,
}

#[derive(Clone, Copy)]
pub(in crate::runner_native) struct OutputLimits {
    pub stdout: usize,
    pub stderr: usize,
}

pub(in crate::runner_native) fn collect_child(
    pid: libc::pid_t,
    pipes: ChildPipes,
    input: &[u8],
    deadline: Instant,
    limits: OutputLimits,
    outcome: &str,
) -> io::Result<Vec<u8>> {
    let mut child = ChildGuard::new(pid);
    let mut stdin = Some(unsafe { File::from_raw_fd(pipes.stdin) });
    let mut stdout = Some(unsafe { File::from_raw_fd(pipes.stdout) });
    let mut stderr = Some(unsafe { File::from_raw_fd(pipes.stderr) });
    for file in [&stdin, &stdout, &stderr].into_iter().flatten() {
        set_nonblocking(file.as_raw_fd())?;
    }

    let mut input_offset = 0;
    let mut output = Vec::new();
    let mut error_output = Vec::new();
    let mut status = None;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native probe exceeded total 30-second deadline",
            ));
        }

        if status.is_none() {
            status = child.try_reap()?;
            if status.is_some() {
                stdin = None;
            }
        }
        write_available(&mut stdin, input, &mut input_offset)?;
        read_available(&mut stdout, &mut output, limits.stdout, "probe stdout")?;
        read_available(
            &mut stderr,
            &mut error_output,
            limits.stderr,
            "probe stderr",
        )?;

        if let Some(wait_status) = status
            && stdout.is_none()
            && stderr.is_none()
        {
            if !libc::WIFEXITED(wait_status) || libc::WEXITSTATUS(wait_status) != 0 {
                return Err(io::Error::other(format!(
                    "{outcome} exited with wait status {wait_status}"
                )));
            }
            return Ok(output);
        }

        let mut poll_fds = [
            libc::pollfd {
                fd: stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLOUT,
                revents: 0,
            },
            libc::pollfd {
                fd: stdout.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
            libc::pollfd {
                fd: stderr.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
        ];
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_ms = remaining.as_millis().clamp(1, 50) as i32;
        let polled = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                timeout_ms,
            )
        };
        if polled < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn write_available(file: &mut Option<File>, input: &[u8], offset: &mut usize) -> io::Result<()> {
    let Some(writer) = file.as_mut() else {
        return Ok(());
    };
    while *offset < input.len() {
        match writer.write(&input[*offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "child stdin closed",
                ));
            }
            Ok(written) => *offset += written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    *file = None;
    Ok(())
}

fn read_available(
    file: &mut Option<File>,
    output: &mut Vec<u8>,
    maximum: usize,
    label: &str,
) -> io::Result<()> {
    let Some(reader) = file.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                *file = None;
                return Ok(());
            }
            Ok(read) => {
                if output
                    .len()
                    .checked_add(read)
                    .is_none_or(|total| total > maximum)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{label} exceeds fixed bound"),
                    ));
                }
                output.extend_from_slice(&buffer[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

struct ChildGuard {
    pid: libc::pid_t,
    reaped: bool,
}

impl ChildGuard {
    fn new(pid: libc::pid_t) -> Self {
        Self { pid, reaped: false }
    }

    fn try_reap(&mut self) -> io::Result<Option<i32>> {
        loop {
            let mut status = 0;
            match unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) } {
                0 => return Ok(None),
                pid if pid == self.pid => {
                    self.reaped = true;
                    return Ok(Some(status));
                }
                -1 => {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
                _ => return Err(io::Error::other("waitpid returned an unexpected child")),
            }
        }
    }

    fn terminate_and_reap(&mut self) {
        if self.reaped {
            return;
        }
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
        }
        loop {
            let mut status = 0;
            match unsafe { libc::waitpid(self.pid, &mut status, 0) } {
                pid if pid == self.pid => {
                    self.reaped = true;
                    return;
                }
                -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => {}
                _ => return,
            }
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate_and_reap();
    }
}
