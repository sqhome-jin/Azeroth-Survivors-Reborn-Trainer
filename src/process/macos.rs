//! macOS process access through libproc and Mach task ports.
//!
//! `task_for_pid` only succeeds when this executable is signed with the
//! `com.apple.security.cs.debugger` entitlement and the game is not protected
//! by the hardened runtime. The release build is signed that way; the error
//! messages below explain the requirement when it is missing.
use std::{
    fs, io,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use mach2::{
    kern_return::{KERN_FAILURE, KERN_PROTECTION_FAILURE, KERN_SUCCESS, kern_return_t},
    mach_port::mach_port_deallocate,
    message::mach_msg_type_number_t,
    port::{MACH_PORT_NULL, mach_port_t},
    traps::{mach_task_self, task_for_pid},
    vm::{mach_vm_read_overwrite, mach_vm_region_recurse, mach_vm_write},
    vm_prot::{VM_PROT_EXECUTE, VM_PROT_READ},
    vm_region::vm_region_submap_info_64,
    vm_statistics::VM_MEMORY_STACK,
    vm_types::{mach_vm_address_t, mach_vm_size_t, natural_t},
};

use super::{GAME_EXE, Games, Session, same_path};

const LAUNCHER: &str = "AzerothSurvivorsLauncher";
const BUNDLE_ID: &str = "com.azreborn.survivors";
const PROC_ALL_PIDS: u32 = 1;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_EXECUTE: u32 = 2;

fn mach_error(code: kern_return_t, action: &str) -> io::Error {
    let detail = match code {
        KERN_FAILURE => "access was refused",
        KERN_PROTECTION_FAILURE => "memory protection refused the access",
        _ => "the kernel returned an error",
    };
    io::Error::other(format!("{action}: {detail} (kern_return {code})."))
}

/// Default location of the game binary installed by the official launcher.
pub fn default_game_path() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/Application Support/Azeroth Survivors")
        .join(GAME_EXE);
    path.is_file().then_some(path)
}

struct Task {
    port: mach_port_t,
    owned: bool,
}

impl Task {
    fn open(session: &Session) -> io::Result<Self> {
        let task = if session.pid == std::process::id() {
            Self {
                port: unsafe { mach_task_self() },
                owned: false,
            }
        } else {
            let mut port = MACH_PORT_NULL;
            let code = unsafe { task_for_pid(mach_task_self(), session.pid as i32, &mut port) };
            if code != KERN_SUCCESS {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "macOS refused access to the game process (kern_return {code}). \
                         The trainer must be signed with the debugger entitlement and run \
                         by an administrator or a member of the _developer group."
                    ),
                ));
            }
            Self { port, owned: true }
        };
        if identity(session.pid)? != *session {
            return Err(io::Error::other("Reborn process identity changed."));
        }
        Ok(task)
    }

    fn read(&self, address: usize, bytes: &mut [u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut count: mach_vm_size_t = 0;
        // The destination is a local slice; the kernel copies into it synchronously.
        let code = unsafe {
            mach_vm_read_overwrite(
                self.port,
                address as mach_vm_address_t,
                bytes.len() as mach_vm_size_t,
                bytes.as_mut_ptr() as mach_vm_address_t,
                &mut count,
            )
        };
        if code != KERN_SUCCESS {
            Err(mach_error(code, "Game memory read failed"))
        } else if count != bytes.len() as mach_vm_size_t {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Partial game memory read.",
            ))
        } else {
            Ok(())
        }
    }

    fn regions(&self) -> impl Iterator<Item = Region> + '_ {
        let mut address: mach_vm_address_t = 0;
        let mut depth: natural_t = 0;
        std::iter::from_fn(move || {
            loop {
                let mut size: mach_vm_size_t = 0;
                let mut info = vm_region_submap_info_64::default();
                let mut count = vm_region_submap_info_64::count();
                // `info` is a correctly sized out-parameter for the requested count.
                let code = unsafe {
                    mach_vm_region_recurse(
                        self.port,
                        &mut address,
                        &mut size,
                        &mut depth,
                        (&mut info as *mut vm_region_submap_info_64).cast(),
                        &mut count,
                    )
                };
                if code != KERN_SUCCESS {
                    return None;
                }
                if info.is_submap != 0 {
                    depth += 1;
                    continue;
                }
                let region = Region {
                    range: address as usize..(address + size) as usize,
                    protection: info.protection,
                    tag: info.user_tag,
                };
                address += size;
                return Some(region);
            }
        })
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if self.owned {
            // The port was received from task_for_pid and is released exactly once.
            unsafe { mach_port_deallocate(mach_task_self(), self.port) };
        }
    }
}

struct Region {
    range: Range<usize>,
    protection: i32,
    tag: u32,
}

impl Region {
    fn readable(&self) -> bool {
        self.protection & VM_PROT_READ != 0
    }
}

fn proc_path(pid: u32) -> io::Result<PathBuf> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length =
        unsafe { libc::proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(String::from_utf8_lossy(&buffer).into_owned()))
}

fn bsd_info(pid: u32) -> io::Result<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // The kernel fills the struct up to `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if written != size {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn identity(pid: u32) -> io::Result<Session> {
    let info = bsd_info(pid)?;
    Ok(Session {
        pid,
        created: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        path: proc_path(pid)?,
    })
}

fn process_name(info: &libc::proc_bsdinfo) -> String {
    let bytes: Vec<u8> = info
        .pbi_name
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn discover() -> io::Result<Games> {
    let needed = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0i32; needed as usize / std::mem::size_of::<i32>() + 64];
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if bytes <= 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate(bytes as usize / std::mem::size_of::<i32>());
    let mut games = Games::default();
    for pid in pids.into_iter().filter(|&pid| pid > 0) {
        let Ok(info) = bsd_info(pid as u32) else {
            continue;
        };
        if info.pbi_status == libc::SZOMB || process_name(&info) != GAME_EXE {
            continue;
        }
        match proc_path(pid as u32) {
            Ok(path) => games.sessions.push(Session {
                pid: pid as u32,
                created: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                path,
            }),
            Err(_) => games.inaccessible += 1,
        }
    }
    Ok(games)
}

/// Starts the official launcher, which updates and then starts the game.
pub fn launch(path: &Path) -> io::Result<Session> {
    let path = path.canonicalize()?;
    if !path.is_file() || path.file_name().is_none_or(|name| name != GAME_EXE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Choose the {GAME_EXE} binary."),
        ));
    }
    if discover()?.running() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Reborn is already running.",
        ));
    }
    let launcher = path.with_file_name(LAUNCHER);
    let mut child = if launcher.is_file() {
        Command::new(&launcher)
            .current_dir(path.parent().expect("canonical executable parent"))
            .spawn()?
    } else {
        Command::new("/usr/bin/open")
            .arg("-b")
            .arg(BUNDLE_ID)
            .spawn()?
    };
    // Reap the launcher when it exits so it never lingers as a zombie.
    thread::spawn(move || {
        let _ = child.wait();
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
        let games = discover()?;
        if let Some(session) = games.select(Some(&path), None) {
            if !same_path(&session.path, &path) {
                return Err(io::Error::other("The launched process identity changed."));
            }
            return Ok(session.clone());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "The launcher did not start Reborn within two minutes.",
    ))
}

pub fn stop(session: &Session) -> io::Result<()> {
    if identity(session.pid)? != *session {
        return Err(io::Error::other("Reborn process identity changed."));
    }
    if unsafe { libc::kill(session.pid as i32, libc::SIGTERM) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub struct Reader(Task);

impl Reader {
    pub fn open(session: &Session) -> io::Result<Self> {
        Task::open(session).map(Self)
    }

    /// Address of the game's Mach-O header, which is the `__TEXT` segment start.
    pub fn module_base(&self, session: &Session) -> io::Result<usize> {
        if identity(session.pid)? != *session {
            return Err(io::Error::other("Reborn process identity changed."));
        }
        let mut header = [0u8; 16];
        for region in self.0.regions() {
            if !region.readable() || region.protection & VM_PROT_EXECUTE == 0 {
                continue;
            }
            if self.0.read(region.range.start, &mut header).is_err()
                || u32::from_le_bytes(header[..4].try_into().unwrap()) != MH_MAGIC_64
                || u32::from_le_bytes(header[12..16].try_into().unwrap()) != MH_EXECUTE
            {
                continue;
            }
            let mut name = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
            let length = unsafe {
                libc::proc_regionfilename(
                    session.pid as i32,
                    region.range.start as u64,
                    name.as_mut_ptr().cast(),
                    name.len() as u32,
                )
            };
            if length > 0 {
                name.truncate(length as usize);
                let mapped = PathBuf::from(String::from_utf8_lossy(&name).into_owned());
                if same_path(&mapped, &session.path) {
                    return Ok(region.range.start);
                }
            }
        }
        Err(io::Error::other(
            "Could not locate the running game module.",
        ))
    }

    pub fn read(&self, address: usize, bytes: &mut [u8]) -> io::Result<()> {
        self.0.read(address, bytes)
    }

    pub fn stacks(&self, session: &Session) -> io::Result<Vec<Range<usize>>> {
        if identity(session.pid)? != *session {
            return Err(io::Error::other("Reborn process identity changed."));
        }
        Ok(self
            .0
            .regions()
            .filter(|region| {
                region.tag == VM_MEMORY_STACK
                    && region.readable()
                    && region.range.len() <= 16 * 1024 * 1024
            })
            .map(|region| region.range)
            .collect())
    }
}

/// Serializes stat writes per game session with an advisory file lock.
pub struct StatWriter {
    task: Task,
    _lock: fs::File,
}

fn writer_lock_path(session: &Session) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};

    let identity_key = format!("{}_{}", session.pid, session.created);
    let digest = format!("{:x}", Sha256::digest(identity_key.as_bytes()));
    let directory = crate::settings::state_directory()?;
    fs::create_dir_all(&directory)?;
    Ok(directory.join(format!("RebornTrainer_{}.lock", &digest[..24])))
}

impl StatWriter {
    pub fn open(session: &Session) -> io::Result<Self> {
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(writer_lock_path(session)?)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(io::Error::other(
                    "Another trainer is currently applying stats to Reborn.",
                ));
            }
            Err(fs::TryLockError::Error(error)) => return Err(error),
        }
        Ok(Self {
            task: Task::open(session)?,
            _lock: lock,
        })
    }

    pub(crate) fn write_value(&self, address: usize, value: &[u8]) -> io::Result<()> {
        // The caller has revalidated this single mapped field and session.
        let code = unsafe {
            mach_vm_write(
                self.task.port,
                address as mach_vm_address_t,
                value.as_ptr() as libc::uintptr_t,
                value.len() as mach_msg_type_number_t,
            )
        };
        if code != KERN_SUCCESS {
            return Err(mach_error(code, "Stat write failed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_acquires_idle_lock_and_serializes_stat_writes() {
        use std::cell::UnsafeCell;

        let session = identity(std::process::id()).unwrap();
        let writer = StatWriter::open(&session).unwrap();
        let competing_session = session.clone();
        assert!(
            std::thread::spawn(move || StatWriter::open(&competing_session).is_err())
                .join()
                .unwrap()
        );
        let values = UnsafeCell::new([17u32, 12]);
        let address = values.get() as usize;
        writer.write_value(address, &18u32.to_le_bytes()).unwrap();
        let mut bytes = [0; 8];
        Reader::open(&session)
            .unwrap()
            .read(address, &mut bytes)
            .unwrap();
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 18);
        assert_eq!(u32::from_le_bytes(bytes[4..].try_into().unwrap()), 12);
        drop(writer);
        let writer = StatWriter::open(&session).unwrap();
        drop(writer);
        let _ = fs::remove_file(writer_lock_path(&session).unwrap());
    }

    #[test]
    fn own_module_base_is_a_mach_o_executable_and_has_stacks() {
        let session = identity(std::process::id()).unwrap();
        let reader = Reader::open(&session).unwrap();
        let base = reader.module_base(&session).unwrap();
        let mut magic = [0u8; 4];
        reader.read(base, &mut magic).unwrap();
        assert_eq!(u32::from_le_bytes(magic), MH_MAGIC_64);
        let local = 0u64;
        let stacks = reader.stacks(&session).unwrap();
        let here = &local as *const u64 as usize;
        assert!(stacks.iter().any(|stack| stack.contains(&here)));
    }
}
