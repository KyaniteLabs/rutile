use std::ffi::{CString, c_char, c_long, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::ptr;
use std::time::Instant;

use crate::runner_native::path_policy::{MeasuredProbe, open_measured_probe};
use crate::runner_native::platform::child_io;

type CFAllocatorRef = *const c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFURLRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFDataRef = *const c_void;
type SecStaticCodeRef = *const c_void;
type SecRequirementRef = *const c_void;

const UTF8: u32 = 0x0800_0100;
const SEC_CS_SIGNING_INFORMATION: u32 = 1 << 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SecurityPins {
    pub designated_requirement: String,
    pub cdhash: [u8; 20],
}

pub(crate) fn read_security_pins(path: &Path) -> io::Result<SecurityPins> {
    unsafe {
        let code = static_code(path)?;
        let result = SecStaticCodeCheckValidityWithErrors(code, 0, ptr::null(), ptr::null_mut());
        if result != 0 {
            CFRelease(code);
            return Err(security("SecStaticCodeCheckValidityWithErrors", result));
        }
        let designated_requirement = copy_designated_requirement(code)?;
        let cdhash = copy_cdhash(code)?;
        CFRelease(code);
        Ok(SecurityPins {
            designated_requirement,
            cdhash,
        })
    }
}

pub(crate) fn verify_security_pins(path: &Path, pins: &SecurityPins) -> io::Result<()> {
    unsafe {
        let code = static_code(path)?;
        let requirement_text = cf_string(&pins.designated_requirement)?;
        let mut requirement: SecRequirementRef = ptr::null();
        let created = SecRequirementCreateWithString(requirement_text, 0, &mut requirement);
        CFRelease(requirement_text);
        if created != 0 {
            CFRelease(code);
            return Err(security("SecRequirementCreateWithString", created));
        }
        let validity = SecStaticCodeCheckValidityWithErrors(code, 0, requirement, ptr::null_mut());
        CFRelease(requirement);
        if validity != 0 {
            CFRelease(code);
            return Err(security("SecStaticCodeCheckValidityWithErrors", validity));
        }
        let actual = copy_cdhash(code)?;
        CFRelease(code);
        if actual != pins.cdhash {
            return Err(invalid("Security framework cdhash mismatch"));
        }
        Ok(())
    }
}

pub(in crate::runner_native) struct NoRace;

pub(in crate::runner_native) trait RaceHooks {
    fn at(&self, _phase: RacePhase, _path: &Path) -> io::Result<()> {
        Ok(())
    }
}

impl RaceHooks for NoRace {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runner_native) enum RacePhase {
    BeforeCopy,
    DuringCopy,
    BeforeSpawn,
}

#[cfg(test)]
pub(in crate::runner_native) struct ReplacePathAt {
    pub phase: RacePhase,
}

#[cfg(test)]
impl RaceHooks for ReplacePathAt {
    fn at(&self, phase: RacePhase, path: &Path) -> io::Result<()> {
        if self.phase == phase {
            fs::rename(path, path.with_extension("raced"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::runner_native) fn copy_verify_posix_spawn<H: RaceHooks>(
    measured: &MeasuredProbe,
    execution_root: &Path,
    pins: &SecurityPins,
    expected_uid: u32,
    hooks: &H,
    arguments: &[&str],
) -> io::Result<()> {
    with_verified_copy(
        measured,
        execution_root,
        pins,
        expected_uid,
        hooks,
        |path| posix_spawn_wait(path, arguments),
    )
}

pub(in crate::runner_native) fn copy_verify_posix_spawn_capture(
    measured: &MeasuredProbe,
    execution_root: &Path,
    pins: &SecurityPins,
    expected_uid: u32,
    input: &[u8],
    limits: child_io::OutputLimits,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let arguments = ["feathermark-runner-probe"];
    with_verified_copy(
        measured,
        execution_root,
        pins,
        expected_uid,
        &NoRace,
        |path| posix_spawn_capture(path, &arguments, input, limits, deadline),
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(in crate::runner_native) fn copy_verify_posix_spawn_capture_for_test(
    measured: &MeasuredProbe,
    execution_root: &Path,
    pins: &SecurityPins,
    expected_uid: u32,
    arguments: &[&str],
    input: &[u8],
    limits: child_io::OutputLimits,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    with_verified_copy(
        measured,
        execution_root,
        pins,
        expected_uid,
        &NoRace,
        |path| posix_spawn_capture(path, arguments, input, limits, deadline),
    )
}

fn with_verified_copy<T, H: RaceHooks>(
    measured: &MeasuredProbe,
    execution_root: &Path,
    pins: &SecurityPins,
    expected_uid: u32,
    hooks: &H,
    spawn: impl FnOnce(&Path) -> io::Result<T>,
) -> io::Result<T> {
    verify_security_pins(&measured_path(measured)?, pins)?;
    hooks.at(RacePhase::BeforeCopy, &measured_path(measured)?)?;
    if !measured.path_still_matches()? {
        return Err(invalid("installed probe changed before immutable copy"));
    }

    let execution_directory = unique_execution_directory(execution_root)?;
    let copy_path = execution_directory.join("probe");
    let result = (|| {
        let mut source = measured.file().try_clone()?;
        source.seek(SeekFrom::Start(0))?;
        let mut copy = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&copy_path)?;
        let copied = io::copy(&mut source, &mut copy)?;
        if copied != measured.length() {
            return Err(invalid("immutable copy length mismatch"));
        }
        copy.sync_all()?;
        fs::set_permissions(&copy_path, fs::Permissions::from_mode(0o500))?;
        copy.sync_all()?;

        hooks.at(RacePhase::DuringCopy, &measured_path(measured)?)?;
        if !measured.path_still_matches()? {
            return Err(invalid("installed probe changed during immutable copy"));
        }
        let copy_measured = open_measured_probe(
            &execution_directory,
            "probe",
            measured.digest(),
            expected_uid,
        )?;
        if copy_measured.length() != measured.length() {
            return Err(invalid("immutable copy length changed"));
        }
        verify_security_pins(&copy_path, pins)?;

        hooks.at(RacePhase::BeforeSpawn, &copy_path)?;
        if !copy_measured.path_still_matches()? {
            return Err(invalid("immutable copy changed before posix_spawn"));
        }
        verify_security_pins(&copy_path, pins)?;
        spawn(&copy_path)
    })();

    let _ = fs::remove_file(&copy_path);
    let _ = File::open(&execution_directory).and_then(|directory| directory.sync_all());
    let _ = fs::remove_dir(&execution_directory);
    let _ = File::open(execution_root).and_then(|directory| directory.sync_all());
    result
}

fn measured_path(measured: &MeasuredProbe) -> io::Result<std::path::PathBuf> {
    // The held descriptor is authoritative. This path is used only for Security.framework
    // validation and is checked against the descriptor identity before and after each use.
    Ok(measured.path().to_path_buf())
}

fn unique_execution_directory(root: &Path) -> io::Result<std::path::PathBuf> {
    for _ in 0..16 {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let path = root.join(format!("request-{}", hex::encode(nonce)));
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                File::open(root)?.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate unique execution directory",
    ))
}

#[cfg(test)]
fn posix_spawn_wait(path: &Path, arguments: &[&str]) -> io::Result<()> {
    if arguments.is_empty() {
        return Err(invalid("posix_spawn requires argv[0]"));
    }
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| invalid("spawn path contains NUL"))?;
    let values: Vec<CString> = arguments
        .iter()
        .map(|argument| CString::new(*argument).map_err(|_| invalid("spawn argument contains NUL")))
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<*mut c_char> = values
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(ptr::null_mut()))
        .collect();
    let environment = CString::new("PATH=/usr/bin:/bin").expect("literal has no NUL");
    let mut environment = [environment.as_ptr().cast_mut(), ptr::null_mut()];
    let mut pid = 0;
    let status = unsafe {
        posix_spawn(
            &mut pid,
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            argv.as_mut_ptr(),
            environment.as_mut_ptr(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    let mut child_status = 0;
    if unsafe { libc::waitpid(pid, &mut child_status, 0) } != pid {
        return Err(io::Error::last_os_error());
    }
    if !libc::WIFEXITED(child_status) || libc::WEXITSTATUS(child_status) != 0 {
        return Err(io::Error::other(format!(
            "copied probe exited with wait status {child_status}"
        )));
    }
    Ok(())
}

fn posix_spawn_capture(
    path: &Path,
    arguments: &[&str],
    input: &[u8],
    limits: child_io::OutputLimits,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    if arguments.is_empty() {
        return Err(invalid("posix_spawn requires argv[0]"));
    }
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| invalid("spawn path contains NUL"))?;
    let values: Vec<CString> = arguments
        .iter()
        .map(|argument| CString::new(*argument).map_err(|_| invalid("spawn argument contains NUL")))
        .collect::<io::Result<_>>()?;
    let mut argv: Vec<*mut c_char> = values
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(ptr::null_mut()))
        .collect();
    let environment = CString::new("PATH=/usr/bin:/bin").expect("literal has no NUL");
    let mut environment = [environment.as_ptr().cast_mut(), ptr::null_mut()];
    let mut input_pipe = [-1; 2];
    let mut output_pipe = [-1; 2];
    let mut error_pipe = [-1; 2];
    if unsafe { libc::pipe(input_pipe.as_mut_ptr()) } != 0
        || unsafe { libc::pipe(output_pipe.as_mut_ptr()) } != 0
        || unsafe { libc::pipe(error_pipe.as_mut_ptr()) } != 0
    {
        let error = io::Error::last_os_error();
        close_pipes(&[input_pipe, output_pipe, error_pipe]);
        return Err(error);
    }
    let mut actions = std::mem::MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
    let initialized = unsafe { posix_spawn_file_actions_init(actions.as_mut_ptr()) };
    if initialized != 0 {
        close_pipes(&[input_pipe, output_pipe, error_pipe]);
        return Err(io::Error::from_raw_os_error(initialized));
    }
    let mut actions = unsafe { actions.assume_init() };
    let setup = unsafe {
        [
            posix_spawn_file_actions_adddup2(&mut actions, input_pipe[0], libc::STDIN_FILENO),
            posix_spawn_file_actions_adddup2(&mut actions, output_pipe[1], libc::STDOUT_FILENO),
            posix_spawn_file_actions_adddup2(&mut actions, error_pipe[1], libc::STDERR_FILENO),
            posix_spawn_file_actions_addclose(&mut actions, input_pipe[0]),
            posix_spawn_file_actions_addclose(&mut actions, input_pipe[1]),
            posix_spawn_file_actions_addclose(&mut actions, output_pipe[0]),
            posix_spawn_file_actions_addclose(&mut actions, output_pipe[1]),
            posix_spawn_file_actions_addclose(&mut actions, error_pipe[0]),
            posix_spawn_file_actions_addclose(&mut actions, error_pipe[1]),
        ]
    };
    if let Some(setup) = setup.into_iter().find(|status| *status != 0) {
        unsafe { posix_spawn_file_actions_destroy(&mut actions) };
        close_pipes(&[input_pipe, output_pipe, error_pipe]);
        return Err(io::Error::from_raw_os_error(setup));
    }
    let mut pid = 0;
    let spawned = unsafe {
        posix_spawn(
            &mut pid,
            path.as_ptr(),
            &actions,
            ptr::null(),
            argv.as_mut_ptr(),
            environment.as_mut_ptr(),
        )
    };
    unsafe { posix_spawn_file_actions_destroy(&mut actions) };
    unsafe {
        libc::close(input_pipe[0]);
        libc::close(output_pipe[1]);
        libc::close(error_pipe[1]);
    }
    if spawned != 0 {
        unsafe {
            libc::close(input_pipe[1]);
            libc::close(output_pipe[0]);
            libc::close(error_pipe[0]);
        }
        return Err(io::Error::from_raw_os_error(spawned));
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
        limits,
        "posix_spawn probe",
    )
}

fn close_pipes(pipes: &[[i32; 2]]) {
    for fd in pipes.iter().flatten().copied().filter(|fd| *fd >= 0) {
        unsafe {
            libc::close(fd);
        }
    }
}

unsafe fn static_code(path: &Path) -> io::Result<SecStaticCodeRef> {
    let path = path
        .to_str()
        .ok_or_else(|| invalid("signed executable path is not UTF-8"))?;
    let url = unsafe {
        CFURLCreateFromFileSystemRepresentation(
            ptr::null(),
            path.as_ptr(),
            path.len() as c_long,
            false,
        )
    };
    if url.is_null() {
        return Err(invalid("CFURLCreateFromFileSystemRepresentation failed"));
    }
    let mut code: SecStaticCodeRef = ptr::null();
    let status = unsafe { SecStaticCodeCreateWithPath(url, 0, &mut code) };
    unsafe { CFRelease(url) };
    if status != 0 || code.is_null() {
        Err(security("SecStaticCodeCreateWithPath", status))
    } else {
        Ok(code)
    }
}

unsafe fn copy_designated_requirement(code: SecStaticCodeRef) -> io::Result<String> {
    let mut requirement: SecRequirementRef = ptr::null();
    let status = unsafe { SecCodeCopyDesignatedRequirement(code, 0, &mut requirement) };
    if status != 0 || requirement.is_null() {
        return Err(security("SecCodeCopyDesignatedRequirement", status));
    }
    let mut text: CFStringRef = ptr::null();
    let status = unsafe { SecRequirementCopyString(requirement, 0, &mut text) };
    unsafe { CFRelease(requirement) };
    if status != 0 || text.is_null() {
        return Err(security("SecRequirementCopyString", status));
    }
    let result = unsafe { string_from_cf(text) };
    unsafe { CFRelease(text) };
    result
}

unsafe fn copy_cdhash(code: SecStaticCodeRef) -> io::Result<[u8; 20]> {
    let mut information: CFDictionaryRef = ptr::null();
    let status = unsafe {
        SecCodeCopySigningInformation(code, SEC_CS_SIGNING_INFORMATION, &mut information)
    };
    if status != 0 || information.is_null() {
        return Err(security("SecCodeCopySigningInformation", status));
    }
    let data =
        unsafe { CFDictionaryGetValue(information, kSecCodeInfoUnique as CFTypeRef) } as CFDataRef;
    if data.is_null() || unsafe { CFDataGetLength(data) } != 20 {
        unsafe { CFRelease(information) };
        return Err(invalid("Security framework returned a non-20-byte cdhash"));
    }
    let mut cdhash = [0_u8; 20];
    let bytes = unsafe { CFDataGetBytePtr(data) };
    if bytes.is_null() {
        unsafe { CFRelease(information) };
        return Err(invalid("Security framework returned a null cdhash"));
    }
    unsafe { ptr::copy_nonoverlapping(bytes, cdhash.as_mut_ptr(), cdhash.len()) };
    unsafe { CFRelease(information) };
    Ok(cdhash)
}

unsafe fn cf_string(value: &str) -> io::Result<CFStringRef> {
    let string = unsafe {
        CFStringCreateWithBytes(
            ptr::null(),
            value.as_ptr(),
            value.len() as c_long,
            UTF8,
            false,
        )
    };
    if string.is_null() {
        Err(invalid("CFStringCreateWithBytes failed"))
    } else {
        Ok(string)
    }
}

unsafe fn string_from_cf(value: CFStringRef) -> io::Result<String> {
    let length = unsafe { CFStringGetLength(value) };
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) }
        .checked_add(1)
        .ok_or_else(|| invalid("Security requirement string is too large"))?;
    let mut buffer = vec![0_u8; capacity as usize];
    if !unsafe { CFStringGetCString(value, buffer.as_mut_ptr().cast::<c_char>(), capacity, UTF8) } {
        return Err(invalid("CFStringGetCString failed"));
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8(buffer[..end].to_vec())
        .map_err(|_| invalid("Security requirement is not UTF-8"))
}

fn security(operation: &str, status: i32) -> io::Error {
    io::Error::other(format!("{operation} failed with OSStatus {status}"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: CFTypeRef);
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: CFAllocatorRef,
        bytes: *const u8,
        length: c_long,
        is_directory: bool,
    ) -> CFURLRef;
    fn CFStringCreateWithBytes(
        allocator: CFAllocatorRef,
        bytes: *const u8,
        length: c_long,
        encoding: u32,
        is_external_representation: bool,
    ) -> CFStringRef;
    fn CFStringGetLength(value: CFStringRef) -> c_long;
    fn CFStringGetMaximumSizeForEncoding(length: c_long, encoding: u32) -> c_long;
    fn CFStringGetCString(
        value: CFStringRef,
        buffer: *mut c_char,
        buffer_size: c_long,
        encoding: u32,
    ) -> bool;
    fn CFDictionaryGetValue(dictionary: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDataGetLength(data: CFDataRef) -> c_long;
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
}

unsafe extern "C" {
    fn posix_spawn(
        pid: *mut libc::pid_t,
        path: *const c_char,
        file_actions: *const libc::posix_spawn_file_actions_t,
        attributes: *const c_void,
        argv: *mut *mut c_char,
        environment: *mut *mut c_char,
    ) -> i32;
    fn posix_spawn_file_actions_init(actions: *mut libc::posix_spawn_file_actions_t) -> i32;
    fn posix_spawn_file_actions_destroy(actions: *mut libc::posix_spawn_file_actions_t) -> i32;
    fn posix_spawn_file_actions_adddup2(
        actions: *mut libc::posix_spawn_file_actions_t,
        from: i32,
        to: i32,
    ) -> i32;
    fn posix_spawn_file_actions_addclose(
        actions: *mut libc::posix_spawn_file_actions_t,
        descriptor: i32,
    ) -> i32;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecCodeInfoUnique: CFStringRef;
    fn SecStaticCodeCreateWithPath(path: CFURLRef, flags: u32, code: *mut SecStaticCodeRef) -> i32;
    fn SecStaticCodeCheckValidityWithErrors(
        code: SecStaticCodeRef,
        flags: u32,
        requirement: SecRequirementRef,
        errors: *mut *const c_void,
    ) -> i32;
    fn SecCodeCopyDesignatedRequirement(
        code: SecStaticCodeRef,
        flags: u32,
        requirement: *mut SecRequirementRef,
    ) -> i32;
    fn SecRequirementCopyString(
        requirement: SecRequirementRef,
        flags: u32,
        text: *mut CFStringRef,
    ) -> i32;
    fn SecRequirementCreateWithString(
        text: CFStringRef,
        flags: u32,
        requirement: *mut SecRequirementRef,
    ) -> i32;
    fn SecCodeCopySigningInformation(
        code: SecStaticCodeRef,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
}
