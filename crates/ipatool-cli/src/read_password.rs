use std::io;
#[cfg(windows)]
use std::ptr;

#[cfg(not(windows))]
pub fn prompt_password(prompt: &str) -> io::Result<String> {
    rpassword::prompt_password(prompt)
}

#[cfg(windows)]
pub fn prompt_password(prompt: &str) -> io::Result<String> {
    let mut output = PromptOutput::open();
    output.write(prompt)?;

    let input = ConsoleInput::open()?;

    let new_mode =
        (input.original_mode | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT) & !ENABLE_ECHO_INPUT;
    if unsafe { SetConsoleMode(input.handle, new_mode) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut data = Vec::new();
    loop {
        let mut buf = [0u16; 512];
        let mut chars_read: u32 = 0;
        let result = unsafe {
            ReadConsoleW(
                input.handle,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut chars_read,
                ptr::null_mut(),
            )
        };

        if result == 0 {
            output.write("\n").ok();
            return Err(io::Error::last_os_error());
        }
        if chars_read == 0 {
            output.write("\n").ok();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of console input",
            ));
        }

        let chunk = &buf[..chars_read as usize];
        data.extend_from_slice(chunk);
        if chunk
            .last()
            .is_some_and(|c| *c == b'\n' as u16 || *c == b'\r' as u16)
        {
            break;
        }
    }

    while data
        .last()
        .is_some_and(|c| *c == b'\n' as u16 || *c == b'\r' as u16)
    {
        data.pop();
    }

    output.write("\n")?;

    let password = String::from_utf16(&data)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16 input"))?;

    Ok(password)
}

#[cfg(windows)]
type Handle = *mut std::ffi::c_void;

#[cfg(windows)]
enum PromptOutput {
    Console(Handle),
    Stderr,
}

#[cfg(windows)]
impl PromptOutput {
    fn open() -> Self {
        match open_console_device("CONOUT$", GENERIC_WRITE) {
            Ok(handle) => Self::Console(handle),
            Err(_) => Self::Stderr,
        }
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        match self {
            Self::Console(handle) => {
                if write_console(*handle, text).is_ok() {
                    return Ok(());
                }

                unsafe {
                    CloseHandle(*handle);
                }
                *self = Self::Stderr;
                self.write(text)
            }
            Self::Stderr => {
                use std::io::Write;

                write!(io::stderr(), "{text}")?;
                io::stderr().flush()
            }
        }
    }
}

#[cfg(windows)]
impl Drop for PromptOutput {
    fn drop(&mut self) {
        if let Self::Console(handle) = self {
            unsafe {
                CloseHandle(*handle);
            }
        }
    }
}

#[cfg(windows)]
struct ConsoleInput {
    handle: Handle,
    original_mode: u32,
    owned: bool,
}

#[cfg(windows)]
impl ConsoleInput {
    fn open() -> io::Result<Self> {
        let std_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        if !is_invalid_handle(std_handle) {
            let mut original_mode = 0;
            if unsafe { GetConsoleMode(std_handle, &mut original_mode) } != 0 {
                return Ok(Self {
                    handle: std_handle,
                    original_mode,
                    owned: false,
                });
            }
        }

        let handle = open_console_device("CONIN$", GENERIC_READ)?;

        let mut original_mode = 0;
        if unsafe { GetConsoleMode(handle, &mut original_mode) } == 0 {
            let err = io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(err);
        }

        Ok(Self {
            handle,
            original_mode,
            owned: true,
        })
    }
}

#[cfg(windows)]
impl Drop for ConsoleInput {
    fn drop(&mut self) {
        unsafe {
            SetConsoleMode(self.handle, self.original_mode);
            if self.owned {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(windows)]
fn is_invalid_handle(handle: Handle) -> bool {
    handle.is_null() || handle as isize == -1
}

#[cfg(windows)]
fn open_console_device(name: &str, access: u32) -> io::Result<Handle> {
    let path = format!("{name}\0").encode_utf16().collect::<Vec<u16>>();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if is_invalid_handle(handle) {
        return Err(io::Error::last_os_error());
    }

    Ok(handle)
}

#[cfg(windows)]
fn write_console(handle: Handle, text: &str) -> io::Result<()> {
    let data = text.encode_utf16().collect::<Vec<u16>>();
    if data.is_empty() {
        return Ok(());
    }

    let mut chars_written: u32 = 0;
    let result = unsafe {
        WriteConsoleW(
            handle,
            data.as_ptr(),
            data.len() as u32,
            &mut chars_written,
            ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(windows)]
const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6u32;
#[cfg(windows)]
const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
#[cfg(windows)]
const ENABLE_LINE_INPUT: u32 = 0x0002;
#[cfg(windows)]
const ENABLE_ECHO_INPUT: u32 = 0x0004;
#[cfg(windows)]
const GENERIC_READ: u32 = 0x80000000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x40000000;
#[cfg(windows)]
const FILE_SHARE_READ: u32 = 0x00000001;
#[cfg(windows)]
const FILE_SHARE_WRITE: u32 = 0x00000002;
#[cfg(windows)]
const OPEN_EXISTING: u32 = 3;

#[cfg(windows)]
unsafe extern "system" {
    fn GetStdHandle(nStdHandle: u32) -> Handle;
    fn GetConsoleMode(hConsoleHandle: Handle, lpMode: *mut u32) -> i32;
    fn SetConsoleMode(hConsoleHandle: Handle, dwMode: u32) -> i32;
    fn ReadConsoleW(
        hConsoleInput: Handle,
        lpBuffer: *mut u16,
        nNumberOfCharsToRead: u32,
        lpNumberOfCharsRead: *mut u32,
        pInputControl: *mut std::ffi::c_void,
    ) -> i32;
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *mut std::ffi::c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: Handle,
    ) -> Handle;
    fn WriteConsoleW(
        hConsoleOutput: Handle,
        lpBuffer: *const u16,
        nNumberOfCharsToWrite: u32,
        lpNumberOfCharsWritten: *mut u32,
        lpReserved: *mut std::ffi::c_void,
    ) -> i32;
    fn CloseHandle(hObject: Handle) -> i32;
}
