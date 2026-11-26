use std::io;

pub(crate) const REFUSAL_EXIT_CODE: i32 = 77;
pub(crate) const REFUSAL_MESSAGE: &str =
    "Run Meta-Harness as a standard user, without root or administrator privileges.";

#[cfg(unix)]
pub(crate) fn elevated() -> io::Result<bool> {
    // SAFETY: These calls only read the current process's real and effective user IDs.
    Ok(unsafe { libc::getuid() == 0 || libc::geteuid() == 0 })
}

#[cfg(windows)]
pub(crate) fn elevated() -> io::Result<bool> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut handle = std::ptr::null_mut();
    // SAFETY: The pseudo-handle refers to this process and the output handle has valid storage.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken returned an owned handle which must be closed once.
    let token = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut result_len = 0;
    // SAFETY: TokenElevation writes a TOKEN_ELEVATION into this initialized buffer.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&raw mut info).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut result_len,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info.TokenIsElevated != 0)
}

#[cfg(windows)]
pub(crate) fn show_if_no_stderr(message: &str) {
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE},
        UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    };
    // SAFETY: Reading standard handles and attaching to an existing console do not transfer ownership.
    let has_stderr = || unsafe {
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        !handle.is_null() && handle != INVALID_HANDLE_VALUE
    };
    if has_stderr() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0 {
        return;
    }
    let caption: Vec<u16> = "Meta-Harness".encode_utf16().chain([0]).collect();
    let text: Vec<u16> = message.encode_utf16().chain([0]).collect();
    // SAFETY: Both strings are terminated and live until the dialog call returns.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}
