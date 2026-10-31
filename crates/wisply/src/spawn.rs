use std::path::Path;

pub fn detached(exe: &Path, args: &[&str], dir: &Path, log: &Path) -> bool {
    sys::detached(exe, args, dir, log)
}

#[cfg(windows)]
mod sys {
    use std::ffi::{OsStr, c_void};
    use std::fs::OpenOptions;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, RawHandle};
    use std::path::Path;
    use std::ptr::{null, null_mut};

    const HIDDEN: u32 = 0x0000_0200 | 0x0800_0000;
    const BREAKAWAY: u32 = 0x0100_0000;
    const EXTENDED_STARTUPINFO: u32 = 0x0008_0000;
    const USE_STD_HANDLES: u32 = 0x0000_0100;
    const HANDLE_LIST: usize = 0x0002_0002;
    const HANDLE_FLAG_INHERIT: u32 = 1;

    #[repr(C)]
    struct StartupInfoEx {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        geometry: [u32; 7],
        flags: u32,
        show_window: u16,
        reserved2_size: u16,
        reserved2: *mut u8,
        std_input: RawHandle,
        std_output: RawHandle,
        std_error: RawHandle,
        attributes: *mut c_void,
    }

    #[repr(C)]
    struct ProcessInfo {
        process: RawHandle,
        thread: RawHandle,
        ids: [u32; 2],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
        fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
        fn UpdateProcThreadAttribute(
            list: *mut c_void,
            flags: u32,
            attribute: usize,
            value: *const c_void,
            size: usize,
            previous: *mut c_void,
            returned: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(
            application: *const u16,
            command: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit: i32,
            flags: u32,
            environment: *const c_void,
            directory: *const u16,
            startup: *const StartupInfoEx,
            info: *mut ProcessInfo,
        ) -> i32;
        fn CloseHandle(handle: RawHandle) -> i32;
    }

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    pub fn detached(exe: &Path, args: &[&str], dir: &Path, log: &Path) -> bool {
        let Ok(log) = OpenOptions::new().create(true).append(true).open(log) else { return false };
        let handle = log.as_raw_handle();
        let mut line = format!("\"{}\"", exe.display());
        for arg in args {
            line.push(' ');
            line.push_str(arg);
        }
        let mut command = wide(OsStr::new(&line));
        let directory = wide(dir.as_os_str());
        let mut started = false;
        unsafe {
            let mut size = 0usize;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
            let mut buffer = vec![0usize; size / size_of::<usize>() + 1];
            let list = buffer.as_mut_ptr().cast::<c_void>();
            if SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) == 0
                || InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0
            {
                return false;
            }
            let handles = [handle];
            let listed = UpdateProcThreadAttribute(
                list,
                0,
                HANDLE_LIST,
                handles.as_ptr().cast(),
                size_of::<RawHandle>(),
                null_mut(),
                null_mut(),
            );
            if listed != 0 {
                let mut startup: StartupInfoEx = zeroed();
                startup.cb = size_of::<StartupInfoEx>() as u32;
                startup.flags = USE_STD_HANDLES;
                startup.std_output = handle;
                startup.std_error = handle;
                startup.attributes = list;
                for flags in [HIDDEN | BREAKAWAY, HIDDEN] {
                    let mut info: ProcessInfo = zeroed();
                    let flags = flags | EXTENDED_STARTUPINFO;
                    let ok = CreateProcessW(
                        null(),
                        command.as_mut_ptr(),
                        null(),
                        null(),
                        1,
                        flags,
                        null(),
                        directory.as_ptr(),
                        &startup,
                        &mut info,
                    );
                    if ok != 0 {
                        CloseHandle(info.process);
                        CloseHandle(info.thread);
                        started = true;
                        break;
                    }
                }
            }
            DeleteProcThreadAttributeList(list);
        }
        started
    }
}

#[cfg(not(windows))]
mod sys {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    unsafe extern "C" {
        fn setsid() -> i32;
        fn close(fd: i32) -> i32;
    }

    pub fn detached(exe: &Path, args: &[&str], dir: &Path, log: &Path) -> bool {
        launch(exe, args, dir, log).is_ok()
    }

    fn launch(exe: &Path, args: &[&str], dir: &Path, log: &Path) -> io::Result<Child> {
        let out = OpenOptions::new().create(true).append(true).open(log)?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(exe);
        cmd.args(args).current_dir(dir).stdin(Stdio::null()).stdout(out).stderr(err);
        unsafe {
            cmd.pre_exec(|| {
                setsid();
                for fd in 3..256 {
                    close(fd);
                }
                Ok(())
            });
        }
        cmd.spawn()
    }
}
