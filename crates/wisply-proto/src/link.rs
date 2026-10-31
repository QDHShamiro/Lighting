use std::io;
use std::path::Path;
use std::time::Duration;

use crate::Conn;

pub struct Link(pub(crate) sys::Pipe);

impl Link {
    pub fn connect(home: &Path) -> Result<Link, Conn> {
        sys::open(home).map(Link)
    }

    pub fn send(&self, msg: &[u8]) -> io::Result<()> {
        self.0.send(msg)
    }

    pub fn recv(&self) -> io::Result<Vec<u8>> {
        self.0.recv(None)
    }

    pub fn recv_within(&self, limit: Duration) -> io::Result<Vec<u8>> {
        self.0.recv(Some(limit))
    }
}

#[cfg(not(windows))]
pub(crate) mod sys {
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::Duration;

    use crate::Conn;

    pub struct Pipe(pub(crate) UnixStream);

    pub fn open(home: &Path) -> Result<Pipe, Conn> {
        UnixStream::connect(crate::address(home)).map(Pipe).map_err(|_| Conn::Missing)
    }

    impl Pipe {
        pub fn send(&self, msg: &[u8]) -> io::Result<()> {
            let size = i32::try_from(msg.len()).map_err(|_| io::Error::other("message too big"))?;
            let mut framed = Vec::with_capacity(msg.len() + 4);
            framed.extend_from_slice(&size.to_be_bytes());
            framed.extend_from_slice(msg);
            (&self.0).write_all(&framed)
        }

        pub fn recv(&self, limit: Option<Duration>) -> io::Result<Vec<u8>> {
            self.0.set_read_timeout(limit)?;
            let got = self.read_message();
            if limit.is_some() {
                self.0.set_read_timeout(None)?;
            }
            got.map_err(|e| if e.kind() == io::ErrorKind::WouldBlock { io::ErrorKind::TimedOut.into() } else { e })
        }

        fn read_message(&self) -> io::Result<Vec<u8>> {
            let mut sock = &self.0;
            let mut head = [0u8; 4];
            sock.read_exact(&mut head)?;
            let size = match i32::from_be_bytes(head) {
                -1 => {
                    let mut big = [0u8; 8];
                    sock.read_exact(&mut big)?;
                    usize::try_from(u64::from_be_bytes(big)).map_err(|_| io::Error::other("message too big"))?
                }
                n => usize::try_from(n).map_err(|_| io::Error::other("bad message size"))?,
            };
            let mut buf = vec![0u8; size];
            sock.read_exact(&mut buf)?;
            Ok(buf)
        }
    }
}

#[cfg(windows)]
pub(crate) mod sys {
    use std::ffi::c_void;
    use std::io;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::time::Duration;

    use crate::Conn;

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    pub(crate) const PIPE_READMODE_MESSAGE: u32 = 2;
    pub(crate) const ERROR_IO_PENDING: u32 = 997;
    pub(crate) const ERROR_MORE_DATA: u32 = 234;
    const ERROR_PIPE_BUSY: u32 = 231;
    const WAIT_TIMEOUT: u32 = 258;
    const INFINITE: u32 = u32::MAX;
    const CHUNK: usize = 64 * 1024;

    #[repr(C)]
    pub(crate) struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: *mut c_void,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn SetNamedPipeHandleState(pipe: *mut c_void, mode: *const u32, count: *const u32, timeout: *const u32) -> i32;
        pub(crate) fn ReadFile(file: *mut c_void, buf: *mut u8, len: u32, read: *mut u32, ov: *mut Overlapped) -> i32;
        pub(crate) fn WriteFile(
            file: *mut c_void,
            buf: *const u8,
            len: u32,
            done: *mut u32,
            ov: *mut Overlapped,
        ) -> i32;
        pub(crate) fn GetOverlappedResult(file: *mut c_void, ov: *mut Overlapped, done: *mut u32, wait: i32) -> i32;
        pub(crate) fn GetOverlappedResultEx(
            file: *mut c_void,
            ov: *mut Overlapped,
            done: *mut u32,
            ms: u32,
            alertable: i32,
        ) -> i32;
        pub(crate) fn CancelIoEx(file: *mut c_void, ov: *mut Overlapped) -> i32;
        fn CreateEventW(security: *const c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
        pub(crate) fn CloseHandle(handle: *mut c_void) -> i32;
        pub(crate) fn GetLastError() -> u32;
    }

    pub struct Pipe(pub(crate) *mut c_void);

    unsafe impl Send for Pipe {}
    unsafe impl Sync for Pipe {}

    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(crate) struct Event(*mut c_void);

    impl Event {
        pub(crate) fn new() -> io::Result<Event> {
            let handle = unsafe { CreateEventW(null(), 1, 0, null()) };
            if handle.is_null() { Err(io::Error::last_os_error()) } else { Ok(Event(handle)) }
        }

        pub(crate) fn overlapped(&self) -> Overlapped {
            Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: self.0 }
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(crate) fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(Some(0)).collect()
    }

    pub fn open(home: &Path) -> Result<Pipe, Conn> {
        open_name(&crate::address(home).to_string_lossy())
    }

    pub(super) fn open_name(name: &str) -> Result<Pipe, Conn> {
        let name = wide(name);
        let access = GENERIC_READ | GENERIC_WRITE;
        let handle =
            unsafe { CreateFileW(name.as_ptr(), access, 0, null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null_mut()) };
        if handle.addr() == usize::MAX {
            return Err(if unsafe { GetLastError() } == ERROR_PIPE_BUSY { Conn::Busy } else { Conn::Missing });
        }
        let pipe = Pipe(handle);
        let mode = PIPE_READMODE_MESSAGE;
        if unsafe { SetNamedPipeHandleState(pipe.0, &mode, null(), null()) } == 0 {
            return Err(Conn::Missing);
        }
        Ok(pipe)
    }

    impl Pipe {
        pub fn send(&self, msg: &[u8]) -> io::Result<()> {
            let len = u32::try_from(msg.len()).map_err(|_| io::Error::other("message too big"))?;
            let event = Event::new()?;
            let mut ov = event.overlapped();
            let mut done = 0u32;
            unsafe {
                if WriteFile(self.0, msg.as_ptr(), len, null_mut(), &mut ov) == 0 && GetLastError() != ERROR_IO_PENDING
                {
                    return Err(io::Error::last_os_error());
                }
                if GetOverlappedResult(self.0, &mut ov, &mut done, 1) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            if done == len { Ok(()) } else { Err(io::Error::other("short write to the wisply daemon")) }
        }

        pub fn recv(&self, limit: Option<Duration>) -> io::Result<Vec<u8>> {
            let ms = limit.map_or(INFINITE, |d| u32::try_from(d.as_millis()).unwrap_or(INFINITE - 1));
            let event = Event::new()?;
            let mut chunk = vec![0u8; CHUNK];
            let mut out = Vec::new();
            loop {
                let mut ov = event.overlapped();
                let mut got = 0u32;
                let more = unsafe {
                    if ReadFile(self.0, chunk.as_mut_ptr(), CHUNK as u32, null_mut(), &mut ov) == 0 {
                        let e = GetLastError();
                        if e != ERROR_IO_PENDING && e != ERROR_MORE_DATA {
                            return Err(io::Error::from_raw_os_error(e as i32));
                        }
                    }
                    if GetOverlappedResultEx(self.0, &mut ov, &mut got, ms, 0) != 0 {
                        false
                    } else {
                        match GetLastError() {
                            ERROR_MORE_DATA => true,
                            WAIT_TIMEOUT => {
                                CancelIoEx(self.0, &mut ov);
                                GetOverlappedResult(self.0, &mut ov, &mut got, 1);
                                return Err(io::ErrorKind::TimedOut.into());
                            }
                            e => return Err(io::Error::from_raw_os_error(e as i32)),
                        }
                    }
                };
                out.extend_from_slice(&chunk[..got as usize]);
                if !more {
                    return if out.is_empty() { Err(io::ErrorKind::UnexpectedEof.into()) } else { Ok(out) };
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::thread;

    use super::sys::{self, CloseHandle, ERROR_MORE_DATA, GetLastError, PIPE_READMODE_MESSAGE, ReadFile, WriteFile};

    const PIPE_ACCESS_DUPLEX: u32 = 3;
    const PIPE_TYPE_MESSAGE: u32 = 4;
    const ERROR_PIPE_CONNECTED: u32 = 535;

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
        fn ConnectNamedPipe(pipe: *mut c_void, ov: *mut sys::Overlapped) -> i32;
        fn FlushFileBuffers(file: *mut c_void) -> i32;
    }

    fn server_read(pipe: *mut c_void) -> Vec<u8> {
        let mut out = Vec::new();
        let mut chunk = vec![0u8; 4096];
        loop {
            let mut got = 0u32;
            let ok = unsafe { ReadFile(pipe, chunk.as_mut_ptr(), 4096, &mut got, null_mut()) };
            out.extend_from_slice(&chunk[..got as usize]);
            if ok != 0 {
                return out;
            }
            assert_eq!(unsafe { GetLastError() }, ERROR_MORE_DATA);
        }
    }

    fn server_write(pipe: *mut c_void, msg: &[u8]) {
        let mut done = 0u32;
        let ok = unsafe { WriteFile(pipe, msg.as_ptr(), msg.len() as u32, &mut done, null_mut()) };
        assert!(ok != 0 && done as usize == msg.len());
    }

    #[test]
    fn whole_messages_cross_the_pipe_both_ways() {
        let name = format!(r"\\.\pipe\wisply-proto-link-{}", std::process::id());
        let mode = PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE;
        let server = unsafe {
            CreateNamedPipeW(sys::wide(&name).as_ptr(), PIPE_ACCESS_DUPLEX, mode, 1, 1 << 16, 1 << 16, 0, null())
        };
        assert_ne!(server.addr(), usize::MAX);
        let big: Vec<u8> = (0..200_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let sent = big.clone();
        let client = thread::spawn(move || {
            let pipe = loop {
                match sys::open_name(&name) {
                    Ok(pipe) => break pipe,
                    Err(_) => thread::sleep(std::time::Duration::from_millis(5)),
                }
            };
            pipe.send(&sent).unwrap();
            pipe.send(b"{\"x\":1}").unwrap();
            (pipe.recv(None).unwrap(), pipe.recv(None).unwrap(), pipe.recv(None).is_err())
        });
        let ok = unsafe { ConnectNamedPipe(server, null_mut()) };
        assert!(ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED);
        assert_eq!(server_read(server), big);
        assert_eq!(server_read(server), b"{\"x\":1}");
        server_write(server, &big);
        server_write(server, b"ok");
        unsafe {
            FlushFileBuffers(server);
            CloseHandle(server);
        }
        let (first, second, closed) = client.join().unwrap();
        assert_eq!(first, big);
        assert_eq!(second, b"ok");
        assert!(closed);
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    use super::Link;

    #[test]
    fn whole_messages_cross_the_socket_both_ways() {
        let home = std::env::temp_dir().join(format!("wisply-proto-link-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let path = home.join(format!("wisply-{}.sock", crate::user()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut head = [0u8; 4];
            sock.read_exact(&mut head).unwrap();
            let mut msg = vec![0u8; i32::from_be_bytes(head) as usize];
            sock.read_exact(&mut msg).unwrap();
            let reply = [&3i32.to_be_bytes()[..], b"abc"].concat();
            sock.write_all(&reply).unwrap();
            msg
        });
        let Ok(link) = Link::connect(&home) else { panic!("no link to the test socket") };
        link.send(b"{\"type\":\"hello\"}").unwrap();
        assert_eq!(link.recv().unwrap(), b"abc");
        assert_eq!(server.join().unwrap(), b"{\"type\":\"hello\"}");
        assert!(link.recv().is_err());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
