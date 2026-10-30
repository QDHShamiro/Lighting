use std::io::{Read, Write};
use std::path::Path;

pub enum Conn {
    Busy,
    Missing,
}

#[cfg(windows)]
pub fn exchange_once(home: &Path, req: &[u8]) -> Result<Vec<u8>, Conn> {
    use std::fs::OpenOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    match OpenOptions::new().read(true).write(true).open(crate::address(home)) {
        Ok(mut pipe) => {
            let mut buf = Vec::new();
            if pipe.write_all(req).is_ok() && pipe.read_to_end(&mut buf).is_ok() {
                return Ok(buf);
            }
            Ok(Vec::new())
        }
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => Err(Conn::Busy),
        Err(_) => Err(Conn::Missing),
    }
}

#[cfg(not(windows))]
pub fn exchange_once(home: &Path, req: &[u8]) -> Result<Vec<u8>, Conn> {
    use std::os::unix::net::UnixStream;
    let mut sock = UnixStream::connect(crate::address(home)).map_err(|_| Conn::Missing)?;
    let Ok(size) = u32::try_from(req.len()) else { return Ok(Vec::new()) };
    let mut framed = Vec::with_capacity(req.len() + 4);
    framed.extend_from_slice(&size.to_be_bytes());
    framed.extend_from_slice(req);
    let mut head = [0u8; 4];
    if sock.write_all(&framed).is_err() || sock.read_exact(&mut head).is_err() {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; u32::from_be_bytes(head) as usize];
    if sock.read_exact(&mut buf).is_err() {
        return Ok(Vec::new());
    }
    Ok(buf)
}
