#[cfg(target_os = "linux")]
use std::time::Duration;
use std::{env, ffi::CString, io::SeekFrom, os::unix::io::RawFd, path::PathBuf};

#[cfg(target_os = "linux")]
use libc::{c_char, statx, statx_timestamp};
use libc::{c_int, iovec, unlink, AT_FDCWD};
use mirrord_protocol::{
    file::{
        OpenFileRequest, OpenFileResponse, OpenOptionsInternal, ReadFileResponse,
        ReadLinkFileRequest, ReadLinkFileResponse, SeekFileResponse, WriteFileResponse,
        XstatFsResponse, XstatResponse,
    },
    ResponseError,
};
use rand::distributions::{Alphanumeric, DistString};
use tracing::{error, trace, Level};

use super::{hooks::FN_OPEN, open_dirs::OPEN_DIRS, *};
#[cfg(target_os = "linux")]
use crate::common::CheckedInto;
use crate::{
    common,
    detour::{Bypass, Detour},
    error::{HookError, HookResult as Result},
};

/// 1 Megabyte. Large read requests can lead to timeouts.
const MAX_READ_SIZE: u64 = 1024 * 1024;

/// Helper macro for checking if the given path should be handled remotely.
/// Uses global [`crate::setup()`].
///
/// Should the file be ignored, this macro exists current context with [`Bypass::IgnoredFile`].
///
/// # Arguments
///
/// * `path` - [`PathBuf`]
/// * `write` - [`bool`], stating whether the file is accessed for writing
macro_rules! ensure_not_ignored {
    ($path:expr, $write:expr) => {
        $crate::setup().file_filter().continue_or_bypass_with(
            $path.to_str().unwrap_or_default(),
            $write,
            || Bypass::ignored_file($path.to_str().unwrap_or_default()),
        )?;
    };
}

macro_rules! check_relative_paths {
    ($path:expr) => {
        if $path.is_relative() {
            Detour::Bypass(Bypass::relative_path($path.to_str().unwrap_or_default()))?
        };
    };
}

macro_rules! remap_path {
    ($path:expr) => {
        $crate::setup().file_remapper().change_path($path)
    };
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RemoteFile {
    pub fd: u64,
    pub path: String,
}

impl RemoteFile {
    pub(crate) fn new(fd: u64, path: String) -> Self {
        Self { fd, path }
    }

    /// Sends a [`OpenFileRequest`] message, opening the file in the agent.
    #[mirrord_layer_macro::instrument(level = "trace")]
    pub(crate) fn remote_open(
        path: PathBuf,
        open_options: OpenOptionsInternal,
    ) -> Detour<OpenFileResponse> {
        let requesting_file = OpenFileRequest { path, open_options };

        let response = common::make_proxy_request_with_response(requesting_file)??;

        Detour::Success(response)
    }

    /// Sends a [`ReadFileRequest`] message, reading the file in the agent.
    ///
    /// Blocking request and wait on already found remote_fd
    #[mirrord_layer_macro::instrument(level = "trace")]
    pub(crate) fn remote_read(remote_fd: u64, read_amount: u64) -> Detour<ReadFileResponse> {
        // Limit read size because if we read too much it can lead to a timeout
        // Seems also that bincode doesn't do well with large buffers
        let read_amount = std::cmp::min(read_amount, MAX_READ_SIZE);
        let reading_file = ReadFileRequest {
            remote_fd,
            buffer_size: read_amount,
        };

        let response = common::make_proxy_request_with_response(reading_file)??;

        Detour::Success(response)
    }

    /// Sends a [`CloseFileRequest`] message, closing the file in the agent.
    #[mirrord_layer_macro::instrument(level = "trace")]
    pub(crate) fn remote_close(fd: u64) -> Result<()> {
        common::make_proxy_request_no_response(CloseFileRequest { fd })?;
        Ok(())
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        // Warning: Don't log from here. This is called when self is removed from OPEN_FILES, so
        // during the whole execution of this function, OPEN_FILES is locked.
        // When emitting logs, sometimes a file `write` operation is required, in order for the
        // operation to complete. The write operation is hooked and at some point tries to lock
        // `OPEN_FILES`, which means the thread deadlocks with itself (we call
        // `OPEN_FILES.lock()?.remove()` and then while still locked, `OPEN_FILES.lock()` again)
        Self::remote_close(self.fd).expect(
            "mirrord failed to send close file message to main layer thread. Error: {err:?}",
        );
    }
}

/// Helper function that retrieves the `remote_fd` (which is generated by
/// `mirrord_agent::util::IndexAllocator`).
fn get_remote_fd(local_fd: RawFd) -> Detour<u64> {
    // don't add a trace here since it causes deadlocks in some cases.
    Detour::Success(
        OPEN_FILES
            .lock()?
            .get(&local_fd)
            .map(|remote_file| remote_file.fd)
            // Bypass if we're not managing the relative part.
            .ok_or(Bypass::LocalFdNotFound(local_fd))?,
    )
}

/// Create temporary local file to get a valid local fd.
#[mirrord_layer_macro::instrument(level = "trace", ret)]
fn create_local_fake_file(remote_fd: u64) -> Detour<RawFd> {
    let random_string = Alphanumeric.sample_string(&mut rand::thread_rng(), 16);
    let file_name = format!("{remote_fd}-{random_string}");
    let file_path = env::temp_dir().join(file_name);
    let file_c_string = CString::new(file_path.to_string_lossy().to_string())?;
    let file_path_ptr = file_c_string.as_ptr();
    let local_file_fd: RawFd = unsafe { FN_OPEN(file_path_ptr, O_RDONLY | O_CREAT) };
    if local_file_fd == -1 {
        // Close the remote file if creating a tmp local file failed and we have an invalid local fd
        close_remote_file_on_failure(remote_fd)?;
        Detour::Error(HookError::LocalFileCreation(remote_fd))
    } else {
        unsafe { unlink(file_path_ptr) };
        Detour::Success(local_file_fd)
    }
}

/// Close the remote file if the call to [`libc::shm_open`] failed and we have an invalid local fd.
#[mirrord_layer_macro::instrument(level = "trace", ret)]
fn close_remote_file_on_failure(fd: u64) -> Result<()> {
    error!("Creating a temporary local file resulted in an error, closing the file remotely!");
    RemoteFile::remote_close(fd)
}

/// Blocking wrapper around `libc::open` call.
///
/// **Bypassed** when trying to load system files, and files from the current working directory
/// (which is different anyways when running in `-agent` context).
///
/// When called for a valid file, it blocks and sends an open file request to be handled by
/// `mirrord-agent`, and waits until it receives an open file response.
///
/// [`open`] is also used by other _open-ish_ functions, and it takes care of **creating** the
/// _local_ and _remote_ file association, plus **inserting** it into the storage for
/// [`OPEN_FILES`].
#[mirrord_layer_macro::instrument(level = Level::TRACE, ret)]
pub(crate) fn open(path: Detour<PathBuf>, open_options: OpenOptionsInternal) -> Detour<RawFd> {
    let path = path?;

    check_relative_paths!(path);

    let path = remap_path!(path);

    ensure_not_ignored!(path, open_options.is_write());

    let OpenFileResponse { fd: remote_fd } = RemoteFile::remote_open(path.clone(), open_options)?;

    // TODO: Need a way to say "open a directory", right now `is_dir` always returns false.
    // This requires having a fake directory name (`/fake`, for example), instead of just converting
    // the fd to a string.
    let local_file_fd = create_local_fake_file(remote_fd)?;

    OPEN_FILES.lock()?.insert(
        local_file_fd,
        Arc::new(RemoteFile::new(remote_fd, path.display().to_string())),
    );

    Detour::Success(local_file_fd)
}

/// creates a directory stream for the `remote_fd` in the agent
#[mirrord_layer_macro::instrument(level = "trace", ret)]
pub(crate) fn fdopendir(fd: RawFd) -> Detour<usize> {
    // usize == ptr size
    // we don't return a pointer to an address that contains DIR
    let remote_file_fd = OPEN_FILES
        .lock()?
        .get(&fd)
        .ok_or(Bypass::LocalFdNotFound(fd))?
        .fd;

    let open_dir_request = FdOpenDirRequest {
        remote_fd: remote_file_fd,
    };

    let OpenDirResponse { fd: remote_dir_fd } =
        common::make_proxy_request_with_response(open_dir_request)??;

    let local_dir_fd = create_local_fake_file(remote_dir_fd)?;
    OPEN_DIRS.insert(local_dir_fd as usize, remote_dir_fd, fd)?;

    // Let it stay in OPEN_FILES, as some functions might use it in comibination with dirfd

    Detour::Success(local_dir_fd as usize)
}

#[mirrord_layer_macro::instrument(level = "trace", ret)]
pub(crate) fn openat(
    fd: RawFd,
    path: Detour<PathBuf>,
    open_options: OpenOptionsInternal,
) -> Detour<RawFd> {
    let path = path?;

    // `openat` behaves the same as `open` when the path is absolute. When called with AT_FDCWD, the
    // call is propagated to `open`.
    if path.is_absolute() || fd == AT_FDCWD {
        let path = remap_path!(path);
        open(Detour::Success(path), open_options)
    } else {
        // Relative path requires special handling, we must identify the relative part (relative to
        // what).
        let remote_fd = get_remote_fd(fd)?;

        let requesting_file = OpenRelativeFileRequest {
            relative_fd: remote_fd,
            path: path.clone(),
            open_options,
        };

        let OpenFileResponse { fd: remote_fd } =
            common::make_proxy_request_with_response(requesting_file)??;

        let local_file_fd = create_local_fake_file(remote_fd)?;

        OPEN_FILES.lock()?.insert(
            local_file_fd,
            Arc::new(RemoteFile::new(remote_fd, path.display().to_string())),
        );

        Detour::Success(local_file_fd)
    }
}

/// Blocking wrapper around [`libc::read`] call.
///
/// **Bypassed** when trying to load system files, and files from the current working directory, see
/// `open`.
pub(crate) fn read(local_fd: RawFd, read_amount: u64) -> Detour<ReadFileResponse> {
    get_remote_fd(local_fd).and_then(|remote_fd| RemoteFile::remote_read(remote_fd, read_amount))
}

/// Helper for dealing with a potential null pointer being passed to `*const iovec` from
/// `readv_detour` and `preadv_detour`.
pub(crate) fn readv(iovs: Option<&[iovec]>) -> Detour<(&[iovec], u64)> {
    let iovs = iovs?;
    let read_size: u64 = iovs.iter().fold(0, |sum, iov| sum + iov.iov_len as u64);

    Detour::Success((iovs, read_size))
}

#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn pread(local_fd: RawFd, buffer_size: u64, offset: u64) -> Detour<ReadFileResponse> {
    // We're only interested in files that are paired with mirrord-agent.
    let remote_fd = get_remote_fd(local_fd)?;

    let reading_file = ReadLimitedFileRequest {
        remote_fd,
        buffer_size,
        start_from: offset,
    };

    let response = common::make_proxy_request_with_response(reading_file)??;

    Detour::Success(response)
}

/// Resolves the symbolic link `path`.
#[mirrord_layer_macro::instrument(level = Level::TRACE, ret)]
pub(crate) fn read_link(path: Detour<PathBuf>) -> Detour<ReadLinkFileResponse> {
    let path = remap_path!(path?);

    check_relative_paths!(path);

    ensure_not_ignored!(path, false);

    let requesting_path = ReadLinkFileRequest { path };

    // `NotImplemented` error here means that the protocol doesn't support it.
    match common::make_proxy_request_with_response(requesting_path)? {
        Ok(response) => Detour::Success(response),
        Err(ResponseError::NotImplemented) => Detour::Bypass(Bypass::NotImplemented),
        Err(fail) => Detour::Error(fail.into()),
    }
}

pub(crate) fn pwrite(local_fd: RawFd, buffer: &[u8], offset: u64) -> Detour<WriteFileResponse> {
    let remote_fd = get_remote_fd(local_fd)?;
    trace!("pwrite: local_fd {local_fd}");

    let writing_file = WriteLimitedFileRequest {
        remote_fd,
        write_bytes: buffer.to_vec(),
        start_from: offset,
    };

    let response = common::make_proxy_request_with_response(writing_file)??;

    Detour::Success(response)
}

#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn lseek(local_fd: RawFd, offset: i64, whence: i32) -> Detour<u64> {
    let remote_fd = get_remote_fd(local_fd)?;

    let seek_from = match whence {
        libc::SEEK_SET => SeekFrom::Start(offset as u64),
        libc::SEEK_CUR => SeekFrom::Current(offset),
        libc::SEEK_END => SeekFrom::End(offset),
        invalid => {
            tracing::warn!(
                "lseek -> potential invalid value {:#?} for whence {:#?}",
                invalid,
                whence
            );
            return Detour::Bypass(Bypass::CStrConversion);
        }
    };

    let seeking_file = SeekFileRequest {
        fd: remote_fd,
        seek_from: seek_from.into(),
    };

    let SeekFileResponse { result_offset } =
        common::make_proxy_request_with_response(seeking_file)??;

    Detour::Success(result_offset)
}

pub(crate) fn write(local_fd: RawFd, write_bytes: Option<Vec<u8>>) -> Detour<isize> {
    let remote_fd = get_remote_fd(local_fd)?;

    let writing_file = WriteFileRequest {
        fd: remote_fd,
        write_bytes: write_bytes.ok_or(Bypass::EmptyBuffer)?,
    };

    let WriteFileResponse { written_amount } =
        common::make_proxy_request_with_response(writing_file)??;
    Detour::Success(written_amount.try_into()?)
}

#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn access(path: Detour<PathBuf>, mode: u8) -> Detour<c_int> {
    let path = path?;

    check_relative_paths!(path);

    let path = remap_path!(path);

    ensure_not_ignored!(path, false);

    let access = AccessFileRequest {
        pathname: path,
        mode,
    };

    let _ = common::make_proxy_request_with_response(access)??;

    Detour::Success(0)
}

/// Original function _flushes_ data from `fd` to disk, but we don't really do any of this
/// for our managed fds, so we just return `0` which means success.
#[mirrord_layer_macro::instrument(level = "trace", ret)]
pub(crate) fn fsync(fd: RawFd) -> Detour<c_int> {
    get_remote_fd(fd)?;
    Detour::Success(0)
}

/// General stat function that can be used for lstat, fstat, stat and fstatat.
/// Note: We treat cases of `AT_SYMLINK_NOFOLLOW_ANY` as `AT_SYMLINK_NOFOLLOW` because even Go does
/// that.
/// rawish_path is Option<Option<&CStr>> because we need to differentiate between null pointer
/// and non existing argument (For error handling)
#[mirrord_layer_macro::instrument(level = "trace", ret)]
pub(crate) fn xstat(
    rawish_path: Option<Detour<PathBuf>>,
    fd: Option<RawFd>,
    follow_symlink: bool,
) -> Detour<XstatResponse> {
    // Can't use map because we need to propagate captured error
    let (path, fd) = match (rawish_path, fd) {
        // fstatat
        (Some(path), Some(fd)) => {
            let path = path?;
            let fd = {
                if fd == AT_FDCWD {
                    check_relative_paths!(path);

                    ensure_not_ignored!(remap_path!(path.clone()), false);
                    None
                } else {
                    Some(get_remote_fd(fd)?)
                }
            };
            (Some(path), fd)
        }
        // lstat/stat
        (Some(path), None) => {
            let path = path?;

            check_relative_paths!(path);

            let path = remap_path!(path);

            ensure_not_ignored!(path, false);
            (Some(path), None)
        }
        // fstat
        (None, Some(fd)) => (None, Some(get_remote_fd(fd)?)),
        // can't happen
        (None, None) => return Detour::Error(HookError::NullPointer),
    };

    let lstat = XstatRequest {
        fd,
        path,
        follow_symlink,
    };

    let response = common::make_proxy_request_with_response(lstat)??;

    Detour::Success(response)
}

/// Logic for the `libc::statx` function.
/// See [manual](https://man7.org/linux/man-pages/man2/statx.2.html) for reference.
///
/// # Warning
///
/// Due to backwards compatibility on the [`mirrord_protocol`] level, we use [`XstatRequest`] to get
/// the remote file metadata.
/// Because of this, we're not able to fill all field of the [`struct@statx`] structure. Missing
/// fields are:
/// 1. [`statx::stx_attributes`]
/// 2. [`statx::stx_ctime`]
/// 3. [`statx::stx_mnt_id`]
/// 4. [`statx::stx_dio_mem_align`] and [`statx::stx_dio_offset_align`]
///
/// Luckily, [`statx::stx_mask`] and [`statx::stx_attributes_mask`] fields allow us to inform the
/// caller about respective fields being skipped.
#[cfg(target_os = "linux")]
pub(crate) fn statx_logic(
    dir_fd: RawFd,
    path_name: *const c_char,
    flags: c_int,
    mask: c_int,
    statx_buf: *mut statx,
) -> Detour<c_int> {
    // SAFETY: we don't check pointers passed as arguments to hooked functions
    let statx_buf = unsafe { statx_buf.as_mut().ok_or(HookError::BadPointer)? };

    if path_name.is_null() {
        return Detour::Error(HookError::BadPointer);
    }
    let path_name: PathBuf = path_name.checked_into()?;

    if (mask & libc::STATX__RESERVED) != 0 {
        return Detour::Error(HookError::BadFlag);
    }

    let (fd, path) = if path_name.is_absolute() {
        ensure_not_ignored!(path_name, false);
        (None, Some(path_name))
    } else if !path_name.as_os_str().is_empty() && dir_fd == libc::AT_FDCWD {
        return Detour::Bypass(Bypass::relative_path(
            path_name.to_str().unwrap_or_default(),
        ));
    } else if !path_name.as_os_str().is_empty() {
        (Some(get_remote_fd(dir_fd)?), Some(path_name))
    } else if (flags & libc::AT_EMPTY_PATH) != 0 {
        (Some(get_remote_fd(dir_fd)?), None)
    } else {
        return Detour::Error(HookError::EmptyPath);
    };

    let response = {
        let fd = fd
            .map(u64::try_from)
            .transpose()
            .map_err(|_| HookError::BadDescriptor)?;
        let follow_symlink = (flags & libc::AT_SYMLINK_NOFOLLOW) == 0;

        let request = XstatRequest {
            fd,
            path,
            follow_symlink,
        };

        common::make_proxy_request_with_response(request)??.metadata
    };

    /// Converts a nanosecond timestamp from
    /// [`MetadataInternal`](mirrord_protocol::file::MetadataInternal) to [`statx_timestamp`]
    /// format.
    fn nanos_to_statx(nanos: i64) -> statx_timestamp {
        let duration = Duration::from_nanos(nanos.try_into().unwrap_or(0));

        statx_timestamp {
            tv_sec: duration.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: duration.subsec_nanos(),
            __statx_timestamp_pad1: [0],
        }
    }

    /// Converts a device id from [`MetadataInternal`](mirrord_protocol::file::MetadataInternal) to
    /// format expected by [`statx`]: (major,minor) number.
    fn device_id_to_statx(id: u64) -> (u32, u32) {
        // SAFETY: these functions only do operations on bits, nothing unsafe here
        unsafe { (libc::major(id), libc::minor(id)) }
    }

    // SAFETY: all-zero statx struct is valid
    *statx_buf = unsafe { std::mem::zeroed() };
    statx_buf.stx_mask = libc::STATX_TYPE
        & libc::STATX_MODE
        & libc::STATX_NLINK
        & libc::STATX_UID
        & libc::STATX_GID
        & libc::STATX_ATIME
        & libc::STATX_MTIME
        & libc::STATX_CTIME
        & libc::STATX_INO
        & libc::STATX_SIZE
        & libc::STATX_BLOCKS;
    statx_buf.stx_attributes_mask = 0;

    statx_buf.stx_blksize = response.block_size.try_into().unwrap_or(u32::MAX);
    statx_buf.stx_nlink = response.hard_links.try_into().unwrap_or(u32::MAX);
    statx_buf.stx_uid = response.user_id;
    statx_buf.stx_gid = response.group_id;
    statx_buf.stx_mode = response.mode as u16; // we only care about the lower half
    statx_buf.stx_ino = response.inode;
    statx_buf.stx_size = response.size;
    statx_buf.stx_blocks = response.blocks;
    statx_buf.stx_atime = nanos_to_statx(response.access_time);
    statx_buf.stx_ctime = nanos_to_statx(response.creation_time);
    statx_buf.stx_mtime = nanos_to_statx(response.modification_time);
    let (major, minor) = device_id_to_statx(response.rdevice_id);
    statx_buf.stx_rdev_major = major;
    statx_buf.stx_rdev_minor = minor;
    let (major, minor) = device_id_to_statx(response.device_id);
    statx_buf.stx_dev_major = major;
    statx_buf.stx_dev_minor = minor;

    Detour::Success(0)
}

#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn xstatfs(fd: RawFd) -> Detour<XstatFsResponse> {
    let fd = get_remote_fd(fd)?;

    let lstatfs = XstatFsRequest { fd };

    let response = common::make_proxy_request_with_response(lstatfs)??;

    Detour::Success(response)
}

#[cfg(target_os = "linux")]
#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn getdents64(fd: RawFd, buffer_size: u64) -> Detour<GetDEnts64Response> {
    // We're only interested in files that are paired with mirrord-agent.
    let remote_fd = get_remote_fd(fd)?;

    let getdents64 = GetDEnts64Request {
        remote_fd,
        buffer_size,
    };

    let response = common::make_proxy_request_with_response(getdents64)??;

    Detour::Success(response)
}

/// Resolves ./ and ../ in the path, and returns an absolute path.
fn absolute_path(path: PathBuf) -> PathBuf {
    use std::path::Component;
    let mut temp_path = PathBuf::new();
    temp_path.push("/");
    for c in path.components() {
        match c {
            Component::RootDir => {}
            Component::CurDir => {}
            Component::Normal(p) => temp_path.push(p),
            Component::ParentDir => {
                temp_path.pop();
            }
            Component::Prefix(_) => {}
        }
    }
    temp_path
}

#[mirrord_layer_macro::instrument(level = "trace")]
pub(crate) fn realpath(path: Detour<PathBuf>) -> Detour<PathBuf> {
    let path = path?;

    check_relative_paths!(path);

    let path = remap_path!(path);

    let realpath = absolute_path(path);

    ensure_not_ignored!(realpath, false);

    // check that file exists
    xstat(Some(Detour::Success(realpath.clone())), None, true)?;

    Detour::Success(realpath)
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::absolute_path;
    #[test]
    fn test_absolute_normal() {
        assert_eq!(
            absolute_path(PathBuf::from("/a/b/c")),
            PathBuf::from("/a/b/c")
        );
        assert_eq!(
            absolute_path(PathBuf::from("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(
            absolute_path(PathBuf::from("/a/b/./c")),
            PathBuf::from("/a/b/c")
        )
    }
}
