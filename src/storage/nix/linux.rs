use anyhow::{Context, Result, anyhow};
use io_uring::{IoUring, opcode, types};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::*;
use std::path::Path;
use sysfs_class::{Block, SysClass};

use crate::storage::{
    AsyncStorageAccess, IoCompletion, IoKind, StorageDetails, StorageError, StorageReadiness,
    StorageRef, StorageType, System,
};

const SYSFS_BLOCK_SIZE: u64 = 512;

impl System {
    pub fn enumerate_storage_devices() -> Result<Vec<StorageRef>> {
        let root = Block::all()?;

        let mut refs = root
            .iter()
            .filter(|d| d.has_device())
            .flat_map(build_device_info)
            .collect::<Vec<_>>();

        refs.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(refs)
    }
}

pub fn open_file_direct<P: AsRef<Path>>(file_path: P, write_access: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(false)
        .append(false)
        .write(write_access)
        .read(true)
        .truncate(false)
        .custom_flags(libc::O_DIRECT /* | libc::O_DSYNC*/) // should be enough in linux 2.6+
        .open(file_path.as_ref())
        .context(format!(
            "Unable to open file-device {}",
            file_path.as_ref().to_str().unwrap_or("?")
        ))
}

/// Opens a device for positional I/O through `io_uring`.
pub fn open_async<P: AsRef<Path>>(path: P, depth: usize) -> Result<Box<dyn AsyncStorageAccess>> {
    let file = open_file_direct(path, true)?;
    let depth = depth.clamp(1, 4096);
    let ring = IoUring::new(u32::try_from(depth).expect("depth is clamped")).context(
        "Unable to create an io_uring instance. The kernel or sandbox may not support it.",
    )?;

    Ok(Box::new(UringAccess {
        file,
        ring,
        depth,
        inflight: 0,
    }))
}

struct UringAccess {
    file: File,
    ring: IoUring,
    depth: usize,
    inflight: usize,
}

impl AsyncStorageAccess for UringAccess {
    fn depth(&self) -> usize {
        self.depth
    }

    fn submit(
        &mut self,
        slot: usize,
        position: u64,
        buffer: &mut [u8],
        kind: IoKind,
    ) -> Result<()> {
        let fd = types::Fd(self.file.as_raw_fd());
        let len = u32::try_from(buffer.len()).context("I/O buffer is too large")?;
        let user_data = u64::try_from(slot).expect("slot fits u64");
        let sqe = match kind {
            IoKind::Read => opcode::Read::new(fd, buffer.as_mut_ptr(), len)
                .offset(position)
                .build()
                .user_data(user_data),
            IoKind::Write => opcode::Write::new(fd, buffer.as_mut_ptr(), len)
                .offset(position)
                .build()
                .user_data(user_data),
        };

        if self.ring.submission().is_full() {
            self.ring
                .submit()
                .context("io_uring submit failed before enqueueing")?;
        }

        // SAFETY: the SQE points at a buffer owned by the caller, which keeps it alive
        // and untouched until the matching completion is reaped.
        unsafe {
            self.ring
                .submission()
                .push(&sqe)
                .map_err(|_| anyhow!("io_uring submission queue is full"))?;
        }

        self.inflight += 1;
        Ok(())
    }

    fn reap(&mut self) -> Result<IoCompletion> {
        let mut completion = self
            .ring
            .completion()
            .next()
            .map(|entry| (entry.user_data(), entry.result()));

        if completion.is_none() {
            self.ring
                .submit_and_wait(1)
                .context("io_uring submit failed")?;

            completion = self
                .ring
                .completion()
                .next()
                .map(|entry| (entry.user_data(), entry.result()));
        }

        let (user_data, result) =
            completion.ok_or_else(|| anyhow!("io_uring returned no completion"))?;

        self.inflight -= 1;

        let result = if result >= 0 {
            Ok(usize::try_from(result).expect("non-negative result fits usize"))
        } else {
            let error = std::io::Error::from_raw_os_error(-result);
            Err(StorageError::from_io(error).into())
        };

        Ok(IoCompletion {
            slot: usize::try_from(user_data).expect("slot fits usize"),
            result,
        })
    }

    fn drain(&mut self) -> Result<()> {
        let mut result = Ok(());

        while self.inflight > 0 {
            if let Err(err) = self.reap() {
                result = Err(err);
                break;
            }
        }

        result
    }

    fn flush(&mut self) -> Result<()> {
        self.file
            .flush()
            .map_err(StorageError::from_io)
            .context("Unable to flush data to the storage")
    }
}

#[allow(dead_code)]
pub fn is_trim_supported(_fd: RawFd) -> bool {
    false
}

pub fn resolve_mount_point<P: AsRef<Path>>(path: P) -> Result<Option<String>> {
    let s = path.as_ref().to_str().unwrap();
    let f = File::open("/proc/mounts")?;
    let reader = BufReader::new(f);

    for line in reader.lines() {
        let l = line?;
        let parts: Vec<&str> = l.split_whitespace().collect();
        if parts[0] == s {
            return Ok(Some(parts[1].to_string()));
        }
    }
    Ok(None)
}

pub fn resolve_fs_label<P: AsRef<Path>>(path: P) -> Result<Option<String>> {
    let labels = std::fs::read_dir("/dev/disk/by-label/")?;

    for entry in labels {
        let label_path = entry.unwrap().path();
        let label_name = label_path.file_name().unwrap();
        let linked_device = std::fs::read_link(&label_path)?;

        if linked_device
            .file_name()
            .unwrap()
            .eq(path.as_ref().file_name().unwrap())
        {
            return Ok(label_name.to_str().map(|s| s.to_owned()));
        }
    }

    Ok(None)
}

fn build_device_info(d: &Block) -> Option<StorageRef> {
    let device_path = format!("/dev/{}", d.path().file_name()?.to_str()?);
    let children = d
        .children()
        .unwrap_or(vec![])
        .iter()
        .flat_map(build_device_info)
        .collect();

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let cpath = CString::new(device_path.as_str()).ok()?;
    unsafe {
        if libc::stat(cpath.as_ptr(), &mut stat) < 0 {
            return None;
        }
    }

    let storage_type = if d.parent_device().is_some() {
        StorageType::Partition
    } else if d.removable().ok()? == 1 {
        StorageType::Removable
    } else {
        StorageType::Fixed
    };

    let details = StorageDetails {
        size: d.size().ok()? * SYSFS_BLOCK_SIZE,
        block_size: stat.st_blksize as usize,
        storage_type,
        mount_point: resolve_mount_point(&device_path).unwrap_or(None),
        label: resolve_fs_label(&device_path).unwrap_or(None),
    };

    Some(StorageRef {
        id: device_path,
        readiness: StorageReadiness::Ready(details),
        children,
    })
}

pub fn unmount(path: &str) -> Result<()> {
    let cpath = CString::new(path)?;
    match unsafe { libc::umount2(cpath.as_ptr(), libc::MNT_FORCE) } {
        0 => Ok(()),
        _ if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) => Ok(()), // not found
        _ => Err(anyhow::Error::new(std::io::Error::last_os_error())
            .context("Failed to unmount a volume")),
    }
}
