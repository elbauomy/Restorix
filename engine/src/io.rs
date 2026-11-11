//! Read-only random access to devices, partitions and image files.
//!
//! Raw devices (`/dev/rdiskN`, `\\.\C:`, `\\.\PhysicalDriveN`) only accept sector-aligned reads, so
//! every read is widened to 4 KiB boundaries. Nothing in this module ever opens a source for writing.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex};

pub trait ReadAt: Send + Sync {
    /// Read up to `buf.len()` bytes at `off`. Returns 0 at or past the end.
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize>;
    fn len(&self) -> u64;

    fn read_full(&self, mut off: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let n = self.read_at(off, buf)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of source"));
            }
            off += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }

    fn read_vec(&self, off: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_full(off, &mut v)?;
        Ok(v)
    }

    /// Like read_vec but returns whatever is available (possibly shorter) instead of failing at the end.
    fn read_upto(&self, off: u64, len: usize) -> Vec<u8> {
        let avail = self.len().saturating_sub(off).min(len as u64) as usize;
        let mut v = vec![0u8; avail];
        let mut got = 0;
        while got < avail {
            match self.read_at(off + got as u64, &mut v[got..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        v.truncate(got);
        v
    }
}

pub type Src = Arc<dyn ReadAt>;

const ALIGN: u64 = 4096;

pub struct RawFile {
    file: File,
    size: u64,
    aligned: bool,
}

impl RawFile {
    fn pread(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            self.file.read_at(buf, off)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            self.file.seek_read(buf, off)
        }
    }
}

impl ReadAt for RawFile {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        if off >= self.size || buf.is_empty() {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.size - off) as usize;
        if !self.aligned {
            return self.pread(off, &mut buf[..want]);
        }
        let start = off & !(ALIGN - 1);
        let end = ((off + want as u64 + ALIGN - 1) & !(ALIGN - 1)).min(self.size.div_ceil(512) * 512);
        if start == off && (want as u64) % 512 == 0 {
            return self.pread(off, &mut buf[..want]);
        }
        let mut tmp = vec![0u8; (end - start) as usize];
        let mut got = 0usize;
        while got < tmp.len() {
            let n = self.pread(start + got as u64, &mut tmp[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        let skip = (off - start) as usize;
        if got <= skip {
            return Ok(0);
        }
        let n = (got - skip).min(want);
        buf[..n].copy_from_slice(&tmp[skip..skip + n]);
        Ok(n)
    }
    fn len(&self) -> u64 {
        self.size
    }
}

/// A window (partition / volume) into another source.
pub struct Slice {
    pub inner: Src,
    pub start: u64,
    pub size: u64,
}

impl ReadAt for Slice {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        if off >= self.size {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(self.size - off) as usize;
        self.inner.read_at(self.start + off, &mut buf[..n])
    }
    fn len(&self) -> u64 {
        self.size
    }
}

/// Small chunk cache for metadata parsing (many small random reads). Large reads bypass it.
pub struct Cached {
    inner: Src,
    map: Mutex<HashMap<u64, Arc<Vec<u8>>>>,
}

const CHUNK: u64 = 64 * 1024;
const MAX_CHUNKS: usize = 1024;

impl Cached {
    pub fn new(inner: Src) -> Src {
        Arc::new(Cached { inner, map: Mutex::new(HashMap::new()) })
    }
    fn chunk(&self, idx: u64) -> io::Result<Arc<Vec<u8>>> {
        if let Some(c) = self.map.lock().unwrap().get(&idx) {
            return Ok(c.clone());
        }
        let data = Arc::new(self.inner.read_upto(idx * CHUNK, CHUNK as usize));
        let mut m = self.map.lock().unwrap();
        if m.len() >= MAX_CHUNKS {
            m.clear();
        }
        m.insert(idx, data.clone());
        Ok(data)
    }
}

impl ReadAt for Cached {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        if buf.len() as u64 > CHUNK * 4 {
            return self.inner.read_at(off, buf);
        }
        let mut done = 0usize;
        while done < buf.len() {
            let pos = off + done as u64;
            let c = self.chunk(pos / CHUNK)?;
            let within = (pos % CHUNK) as usize;
            if within >= c.len() {
                break;
            }
            let n = (c.len() - within).min(buf.len() - done);
            buf[done..done + n].copy_from_slice(&c[within..within + n]);
            done += n;
        }
        Ok(done)
    }
    fn len(&self) -> u64 {
        self.inner.len()
    }
}

/// In-memory source, used by tests.
pub struct MemSrc(pub Vec<u8>);
impl ReadAt for MemSrc {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        if off >= self.0.len() as u64 {
            return Ok(0);
        }
        let o = off as usize;
        let n = buf.len().min(self.0.len() - o);
        buf[..n].copy_from_slice(&self.0[o..o + n]);
        Ok(n)
    }
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
}

pub fn is_device_path(path: &str) -> bool {
    path.starts_with("/dev/") || path.starts_with(r"\\.\") || path.starts_with(r"\\?\")
}

/// Open a device or image read-only.
pub fn open_source(path: &str) -> anyhow::Result<Src> {
    let device = is_device_path(path);
    #[cfg(target_os = "macos")]
    let path = &if let Some(rest) = path.strip_prefix("/dev/disk") {
        format!("/dev/rdisk{rest}") // character device: much faster, no buffer cache
    } else {
        path.to_string()
    };

    let file = match open_plain(path) {
        Ok(f) => f,
        #[cfg(target_os = "macos")]
        Err(e) if device && e.kind() == io::ErrorKind::PermissionDenied => mac::authopen(path).map_err(|e2| {
            anyhow::anyhow!("Administrator access to {path} was not granted ({e2}). Raw disk access needs your password.")
        })?,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            anyhow::bail!(
                "Permission denied opening {path}. {}",
                if cfg!(windows) { "Run Restorix as Administrator." } else { "Run with administrator rights." }
            )
        }
        Err(e) => anyhow::bail!("Cannot open {path}: {e}"),
    };

    let size = if device { device_size(&file)? } else { file.metadata()?.len() };
    if size == 0 {
        anyhow::bail!("{path} reports a size of 0 bytes");
    }
    Ok(Arc::new(RawFile { file, size, aligned: device }))
}

fn open_plain(path: &str) -> io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE so mounted volumes can be read.
        std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::OpenOptions::new().read(true).open(path)
    }
}

#[cfg(target_os = "macos")]
fn device_size(file: &File) -> anyhow::Result<u64> {
    use std::os::unix::io::AsRawFd;
    const DKIOCGETBLOCKSIZE: libc::c_ulong = 0x4004_6418;
    const DKIOCGETBLOCKCOUNT: libc::c_ulong = 0x4008_6419;
    let mut bs: u32 = 0;
    let mut bc: u64 = 0;
    unsafe {
        if libc::ioctl(file.as_raw_fd(), DKIOCGETBLOCKSIZE, &mut bs) != 0
            || libc::ioctl(file.as_raw_fd(), DKIOCGETBLOCKCOUNT, &mut bc) != 0
        {
            anyhow::bail!("cannot query device size: {}", io::Error::last_os_error());
        }
    }
    Ok(bs as u64 * bc)
}

#[cfg(windows)]
fn device_size(file: &File) -> anyhow::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;
    let mut len: i64 = 0;
    let mut ret: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            IOCTL_DISK_GET_LENGTH_INFO,
            std::ptr::null(),
            0,
            &mut len as *mut i64 as _,
            8,
            &mut ret,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        anyhow::bail!("cannot query device size: {}", io::Error::last_os_error());
    }
    Ok(len as u64)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn device_size(file: &File) -> anyhow::Result<u64> {
    use std::io::{Seek, SeekFrom};
    let mut f = file.try_clone()?;
    Ok(f.seek(SeekFrom::End(0))?)
}

#[cfg(target_os = "macos")]
pub mod mac {
    //! `/usr/libexec/authopen` is Apple's supported way for a non-root app to read a raw disk: it asks
    //! for an administrator password and hands back an open file descriptor over a UNIX socket.
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::{Command, Stdio};

    pub fn authopen(path: &str) -> io::Result<File> {
        let mut fds = [0i32; 2];
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let ours = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let theirs = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let mut child = Command::new("/usr/libexec/authopen")
            .arg("-stdoutpipe")
            .arg(path)
            .stdout(Stdio::from(theirs))
            .stderr(Stdio::null())
            .spawn()?;

        let mut data = [0u8; 256];
        let mut iov = libc::iovec { iov_base: data.as_mut_ptr() as *mut _, iov_len: data.len() };
        let mut cbuf = [0u8; 128];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr() as *mut _;
        msg.msg_controllen = cbuf.len() as _;
        let n = unsafe { libc::recvmsg(ours.as_raw_fd(), &mut msg, 0) };
        let status = child.wait()?;
        if n <= 0 || !status.success() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "authorization cancelled or denied"));
        }
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if cmsg.is_null() || (*cmsg).cmsg_level != libc::SOL_SOCKET || (*cmsg).cmsg_type != libc::SCM_RIGHTS {
                return Err(io::Error::new(io::ErrorKind::Other, "authopen returned no descriptor"));
            }
            let fd = *(libc::CMSG_DATA(cmsg) as *const i32);
            Ok(File::from_raw_fd(fd))
        }
    }
}
