use std::path::Path;

const SKIP: &[&str] = &[".obsidian", ".obsidian-mcp", ".trash", "templates", ".git"];

pub fn matters(rel: &str) -> bool {
    let rel = rel.replace('\\', "/").to_lowercase();
    let first = rel.split('/').next().unwrap_or("");
    if SKIP.contains(&first) {
        return false;
    }
    let name = rel.rsplit('/').next().unwrap_or("");
    name.ends_with(".md") || !name.contains('.')
}

pub fn watch(dir: &Path, changed: impl Fn() + Send + 'static) -> bool {
    sys::watch(dir, Box::new(changed))
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::RawHandle;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::thread;

    use super::matters;

    const LIST_DIRECTORY: u32 = 1;
    const SHARE_ALL: u32 = 7;
    const OPEN_EXISTING: u32 = 3;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const CHANGES: u32 = 0x1 | 0x2 | 0x8 | 0x10;
    const BUF_WORDS: usize = 16 * 1024;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const c_void,
            disposition: u32,
            flags: u32,
            template: RawHandle,
        ) -> RawHandle;
        fn ReadDirectoryChangesW(
            dir: RawHandle,
            buf: *mut c_void,
            len: u32,
            subtree: i32,
            filter: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
            routine: *const c_void,
        ) -> i32;
        fn CloseHandle(handle: RawHandle) -> i32;
    }

    struct Dir(RawHandle);

    unsafe impl Send for Dir {}

    impl Drop for Dir {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn names(buf: &[u32], len: usize) -> Vec<String> {
        let bytes: Vec<u8> = buf.iter().flat_map(|w| w.to_le_bytes()).take(len).collect();
        let word = |at: usize| {
            bytes
                .get(at..at + 4)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map_or(0, |b| u32::from_le_bytes(b) as usize)
        };
        let mut out = Vec::new();
        let mut at = 0;
        while at + 12 <= bytes.len() {
            let (next, size) = (word(at), word(at + 8));
            let Some(raw) = bytes.get(at + 12..at + 12 + size) else { break };
            let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            out.push(String::from_utf16_lossy(&units));
            if next == 0 {
                break;
            }
            at += next;
        }
        out
    }

    pub fn watch(dir: &Path, changed: Box<dyn Fn() + Send>) -> bool {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let handle = unsafe {
            CreateFileW(wide.as_ptr(), LIST_DIRECTORY, SHARE_ALL, null(), OPEN_EXISTING, BACKUP_SEMANTICS, null_mut())
        };
        if handle.is_null() || handle as isize == -1 {
            return false;
        }
        let dir = Dir(handle);
        let work = move || {
            let mut buf = vec![0u32; BUF_WORDS];
            loop {
                let mut got = 0u32;
                let ok = unsafe {
                    ReadDirectoryChangesW(
                        dir.0,
                        buf.as_mut_ptr().cast(),
                        (BUF_WORDS * 4) as u32,
                        1,
                        CHANGES,
                        &mut got,
                        null_mut(),
                        null(),
                    )
                };
                if ok == 0 {
                    return;
                }
                if got == 0 || names(&buf, got as usize).iter().any(|n| matters(n)) {
                    changed();
                }
            }
        };
        thread::Builder::new().name("vault-watch".into()).spawn(work).is_ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn change_records_are_read() {
            let mut bytes = Vec::new();
            let first: Vec<u8> = "a.md".encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend_from_slice(&20u32.to_le_bytes());
            bytes.extend_from_slice(&3u32.to_le_bytes());
            bytes.extend_from_slice(&(first.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&first);
            let second: Vec<u8> = r".obsidian\w.json".encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&3u32.to_le_bytes());
            bytes.extend_from_slice(&(second.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&second);
            let len = bytes.len();
            bytes.resize(len.div_ceil(4) * 4, 0);
            let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
            assert_eq!(names(&words, len), ["a.md", r".obsidian\w.json"]);
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::Path;

    pub fn watch(_dir: &Path, _changed: Box<dyn Fn() + Send>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_notes_and_folders_matter() {
        assert!(matters("Rules\\brave.md") && matters("Projects/Shop") && matters("a.MD"));
        assert!(!matters(".obsidian/workspace.json") && !matters(".trash/x.md") && !matters("Bilder/a.png"));
    }
}
