use std::io;
use std::path::Path;

use crate::Link;

pub struct Listener(sys::Server);

impl Listener {
    pub fn bind(home: &Path) -> io::Result<Listener> {
        sys::Server::bind(home).map(Listener)
    }

    pub fn accept(&mut self) -> io::Result<Link> {
        self.0.accept().map(Link)
    }
}

fn taken() -> io::Error {
    io::Error::new(io::ErrorKind::AddrInUse, "another wisply daemon already serves this home")
}

#[cfg(not(windows))]
mod sys {
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    use crate::link::sys::Pipe;

    pub struct Server(UnixListener);

    impl Server {
        pub fn bind(home: &Path) -> io::Result<Server> {
            let path = crate::address(home);
            if UnixStream::connect(&path).is_ok() {
                return Err(super::taken());
            }
            fs::create_dir_all(home)?;
            fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
            let _ = fs::remove_file(&path);
            let listener = UnixListener::bind(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            Ok(Server(listener))
        }

        pub fn accept(&mut self) -> io::Result<Pipe> {
            self.0.accept().map(|(sock, _)| Pipe(sock))
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::io;
    use std::mem::size_of;
    use std::path::Path;
    use std::ptr::null_mut;

    use crate::link::sys::{
        CloseHandle, ERROR_IO_PENDING, Event, GetLastError, GetOverlappedResult, Overlapped, PIPE_READMODE_MESSAGE,
        Pipe, wide,
    };

    const PIPE_ACCESS_DUPLEX: u32 = 3;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_MESSAGE: u32 = 4;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const BUFFER: u32 = 64 * 1024;
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const TOKEN_QUERY: u32 = 8;
    const TOKEN_USER: u32 = 1;
    const SDDL_REVISION_1: u32 = 1;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            instances: u32,
            out_size: u32,
            in_size: u32,
            timeout: u32,
            security: *const c_void,
        ) -> *mut c_void;
        fn ConnectNamedPipe(pipe: *mut c_void, ov: *mut Overlapped) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(token: *mut c_void, class: u32, info: *mut c_void, len: u32, needed: *mut u32) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    struct Local(*mut c_void);

    impl Drop for Local {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0) };
        }
    }

    pub struct Server {
        name: Vec<u16>,
        security: Local,
        next: Option<Pipe>,
    }

    unsafe impl Send for Server {}

    impl Server {
        pub fn bind(home: &Path) -> io::Result<Server> {
            let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?);
            let mut descriptor = null_mut();
            let made = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide(&sddl).as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    null_mut(),
                )
            };
            if made == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut server =
                Server { name: wide(&crate::address(home).to_string_lossy()), security: Local(descriptor), next: None };
            server.next = Some(server.instance(true)?);
            Ok(server)
        }

        fn instance(&self, first: bool) -> io::Result<Pipe> {
            let attrs = SecurityAttributes {
                length: size_of::<SecurityAttributes>() as u32,
                descriptor: self.security.0,
                inherit: 0,
            };
            let open =
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
            let mode = PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS;
            let handle = unsafe {
                CreateNamedPipeW(
                    self.name.as_ptr(),
                    open,
                    mode,
                    PIPE_UNLIMITED_INSTANCES,
                    BUFFER,
                    BUFFER,
                    0,
                    (&raw const attrs).cast(),
                )
            };
            if handle.addr() != usize::MAX {
                return Ok(Pipe(handle));
            }
            let code = unsafe { GetLastError() };
            if first && code == ERROR_ACCESS_DENIED {
                return Err(super::taken());
            }
            Err(io::Error::from_raw_os_error(code as i32))
        }

        pub fn accept(&mut self) -> io::Result<Pipe> {
            let pipe = match self.next.take() {
                Some(pipe) => pipe,
                None => self.instance(false)?,
            };
            let connected = connect(&pipe);
            self.next = self.instance(false).ok();
            connected.map(|()| pipe)
        }
    }

    fn connect(pipe: &Pipe) -> io::Result<()> {
        let event = Event::new()?;
        let mut ov = event.overlapped();
        let mut done = 0u32;
        unsafe {
            if ConnectNamedPipe(pipe.0, &mut ov) != 0 {
                return Ok(());
            }
            match GetLastError() {
                ERROR_PIPE_CONNECTED => Ok(()),
                ERROR_IO_PENDING => {
                    if GetOverlappedResult(pipe.0, &mut ov, &mut done, 1) != 0 {
                        Ok(())
                    } else {
                        Err(io::Error::last_os_error())
                    }
                }
                code => Err(io::Error::from_raw_os_error(code as i32)),
            }
        }
    }

    fn user_sid() -> io::Result<String> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut info = [0usize; 64];
            let mut needed = 0u32;
            let size = size_of::<[usize; 64]>() as u32;
            let read = GetTokenInformation(token, TOKEN_USER, info.as_mut_ptr().cast(), size, &mut needed);
            let failure = io::Error::last_os_error();
            CloseHandle(token);
            if read == 0 {
                return Err(failure);
            }
            let mut text = null_mut();
            if ConvertSidToStringSidW(info[0] as *mut c_void, &mut text) == 0 {
                return Err(io::Error::last_os_error());
            }
            let text = Local(text.cast());
            let chars = text.0.cast::<u16>();
            let len = (0..).take_while(|&i| *chars.add(i) != 0).count();
            Ok(String::from_utf16_lossy(std::slice::from_raw_parts(chars, len)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::Listener;
    use crate::Link;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let base = if cfg!(windows) { std::env::temp_dir() } else { std::path::PathBuf::from("/tmp") };
        base.join(format!("ltp-{tag}-{}", std::process::id()))
    }

    #[test]
    fn a_listener_serves_many_clients_and_owns_its_address() {
        let home = scratch("listen");
        std::fs::create_dir_all(&home).unwrap();
        let mut listener = Listener::bind(&home).unwrap();
        let path = home.clone();
        let clients = thread::spawn(move || {
            (0..3u8)
                .map(|i| {
                    let Ok(link) = Link::connect(&path) else { panic!("no link to the listener") };
                    link.send(&[b'a' + i; 3]).unwrap();
                    link.recv().unwrap()
                })
                .collect::<Vec<_>>()
        });
        for _ in 0..3 {
            let link = listener.accept().unwrap();
            let msg = link.recv_within(Duration::from_secs(5)).unwrap();
            link.send(&[msg, b"!".to_vec()].concat()).unwrap();
        }
        assert_eq!(clients.join().unwrap(), [b"aaa!".to_vec(), b"bbb!".to_vec(), b"ccc!".to_vec()]);
        assert_eq!(Listener::bind(&home).err().map(|e| e.kind()), Some(std::io::ErrorKind::AddrInUse));
        drop(listener);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_silent_client_times_out() {
        let home = scratch("quiet");
        std::fs::create_dir_all(&home).unwrap();
        let mut listener = Listener::bind(&home).unwrap();
        let path = home.clone();
        let client = thread::spawn(move || {
            let Ok(link) = Link::connect(&path) else { panic!("no link to the listener") };
            thread::sleep(Duration::from_millis(600));
            drop(link);
        });
        let link = listener.accept().unwrap();
        let err = link.recv_within(Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        client.join().unwrap();
        drop(listener);
        let _ = std::fs::remove_dir_all(&home);
    }
}
