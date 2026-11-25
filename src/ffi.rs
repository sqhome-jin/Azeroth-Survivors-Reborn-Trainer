//! Panic-safe C ABI for the SwiftUI host.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::{
    ffi::CStr,
    io,
    os::raw::{c_char, c_int},
    path::PathBuf,
    ptr, slice,
};

use serde_json::Value;

use crate::backend::{Backend, JobId, ProofId, SessionId, parse_targets};
use crate::settings::SettingsDto;

pub type RtStatus = c_int;

pub const RT_OK: RtStatus = 0;
pub const RT_ERR: RtStatus = 1;
pub const RT_EMPTY: RtStatus = 2;

#[repr(C)]
pub struct RtBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl RtBuffer {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut boxed = bytes.into_boxed_slice();
        let data = boxed.as_mut_ptr();
        let len = boxed.len();
        std::mem::forget(boxed);
        Self { data, len }
    }

    fn from_json(value: &Value) -> Result<Self, io::Error> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        Ok(Self::from_bytes(bytes))
    }

    fn from_str(text: &str) -> Self {
        Self::from_bytes(text.as_bytes().to_vec())
    }
}

fn catch<T>(f: impl FnOnce() -> Result<T, io::Error>) -> Result<T, io::Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => Err(io::Error::other("Backend panicked.")),
    }
}

fn write_error(out: *mut RtBuffer, error: io::Error) {
    if out.is_null() {
        return;
    }
    let payload = serde_json::json!({
        "code": match error.kind() {
            io::ErrorKind::Interrupted => "cancelled",
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::InvalidInput => "invalid_input",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "failed",
        },
        "message": error.to_string(),
    });
    if let Ok(buffer) = RtBuffer::from_json(&payload) {
        unsafe {
            *out = buffer;
        }
    }
}

fn write_ok_json(out: *mut RtBuffer, value: &Value) -> RtStatus {
    match RtBuffer::from_json(value) {
        Ok(buffer) => {
            if !out.is_null() {
                unsafe {
                    *out = buffer;
                }
            }
            RT_OK
        }
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

fn write_ok_text(out: *mut RtBuffer, text: &str) -> RtStatus {
    if !out.is_null() {
        unsafe {
            *out = RtBuffer::from_str(text);
        }
    }
    RT_OK
}

fn read_cstr<'a>(pointer: *const c_char) -> Result<&'a str, io::Error> {
    if pointer.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Null string pointer.",
        ));
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid UTF-8 string."))
}

fn read_json(pointer: *const u8, len: usize) -> Result<Value, io::Error> {
    if pointer.is_null() && len != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Null JSON pointer.",
        ));
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(pointer, len) }
    };
    serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))
}

fn backend<'a>(handle: *mut Backend) -> Result<&'a mut Backend, io::Error> {
    if handle.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Null backend handle.",
        ));
    }
    Ok(unsafe { &mut *handle })
}

fn write_job(out_job: *mut u64, job_id: u64) -> RtStatus {
    if !out_job.is_null() {
        unsafe {
            *out_job = job_id;
        }
    }
    RT_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_backend_create() -> *mut Backend {
    match catch(|| Ok(Box::new(Backend::create()))) {
        Ok(backend) => Box::into_raw(backend),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_backend_destroy(handle: *mut Backend) {
    if handle.is_null() {
        return;
    }
    let _ = catch(|| {
        unsafe {
            drop(Box::from_raw(handle));
        }
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_buffer_free(buffer: RtBuffer) {
    if buffer.data.is_null() || buffer.len == 0 {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(
            buffer.data,
            buffer.len,
        )));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_poll_event(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    let result = catch(|| {
        let backend = backend(handle)?;
        Ok(backend.poll_event())
    });
    match result {
        Ok(Some(event)) => match serde_json::to_value(event) {
            Ok(value) => write_ok_json(out, &value),
            Err(error) => {
                write_error(out, io::Error::other(error));
                RT_ERR
            }
        },
        Ok(None) => RT_EMPTY,
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_job_cancel(handle: *mut Backend, job_id: u64) -> RtStatus {
    match catch(|| {
        backend(handle)?.cancel(JobId(job_id));
        Ok(())
    }) {
        Ok(()) => RT_OK,
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_proof_release(handle: *mut Backend, proof_id: u64) -> RtStatus {
    match catch(|| {
        backend(handle)?.release_proof(ProofId(proof_id));
        Ok(())
    }) {
        Ok(()) => RT_OK,
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_version(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| Ok(backend(handle)?.version())) {
        Ok(version) => write_ok_text(out, &version),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_settings_load(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| {
        let dto = backend(handle)?.load_settings()?;
        serde_json::to_value(dto).map_err(io::Error::other)
    }) {
        Ok(value) => write_ok_json(out, &value),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_settings_save(
    handle: *mut Backend,
    json: *const u8,
    len: usize,
    out: *mut RtBuffer,
) -> RtStatus {
    match catch(|| {
        let value = read_json(json, len)?;
        let dto: SettingsDto = serde_json::from_value(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        backend(handle)?.save_settings(dto)?;
        Ok(Value::Null)
    }) {
        Ok(value) => write_ok_json(out, &value),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_settings_path(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| {
        backend(handle)?
            .settings_path()
            .map(|path| path.display().to_string())
    }) {
        Ok(path) => write_ok_text(out, &path),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_state_directory(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| {
        backend(handle)?
            .state_directory()
            .map(|path| path.display().to_string())
    }) {
        Ok(path) => write_ok_text(out, &path),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_process_default_path(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| Ok(backend(handle)?.default_game_path())) {
        Ok(Some(path)) => write_ok_text(out, &path),
        Ok(None) => RT_EMPTY,
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_process_discover(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| backend(handle)?.discover_sync()) {
        Ok(value) => write_ok_json(out, &value),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_process_launch(
    handle: *mut Backend,
    path: *const c_char,
    out_job: *mut u64,
) -> RtStatus {
    match catch(|| {
        let path = PathBuf::from(read_cstr(path)?);
        Ok(backend(handle)?.launch_async(path).0)
    }) {
        Ok(job_id) => write_job(out_job, job_id),
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_process_stop(
    handle: *mut Backend,
    session_id: u64,
    out_job: *mut u64,
) -> RtStatus {
    match catch(|| Ok(backend(handle)?.stop_async(SessionId(session_id)).0)) {
        Ok(job_id) => write_job(out_job, job_id),
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_hunt_find(
    handle: *mut Backend,
    session_id: u64,
    out_job: *mut u64,
) -> RtStatus {
    match catch(|| Ok(backend(handle)?.find_async(SessionId(session_id)).0)) {
        Ok(job_id) => write_job(out_job, job_id),
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_hunt_apply(
    handle: *mut Backend,
    proof_id: u64,
    json: *const u8,
    len: usize,
    out_job: *mut u64,
) -> RtStatus {
    match catch(|| {
        let targets = parse_targets(&read_json(json, len)?)?;
        Ok(backend(handle)?.apply_async(ProofId(proof_id), targets).0)
    }) {
        Ok(job_id) => write_job(out_job, job_id),
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_update_check(
    handle: *mut Backend,
    current: *const c_char,
    out_job: *mut u64,
) -> RtStatus {
    match catch(|| {
        let current = read_cstr(current)?.to_owned();
        Ok(backend(handle)?.update_check_async(current).0)
    }) {
        Ok(job_id) => write_job(out_job, job_id),
        Err(_) => RT_ERR,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_diagnostics_paths(handle: *mut Backend, out: *mut RtBuffer) -> RtStatus {
    match catch(|| backend(handle)?.diagnostics_paths()) {
        Ok(value) => write_ok_json(out, &value),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_diagnostics_recent_lines(
    handle: *mut Backend,
    out: *mut RtBuffer,
) -> RtStatus {
    match catch(|| backend(handle)?.recent_log_lines()) {
        Ok(text) => write_ok_text(out, &text),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_stats_metadata(
    handle: *mut Backend,
    chinese: c_int,
    out: *mut RtBuffer,
) -> RtStatus {
    match catch(|| Ok(backend(handle)?.stats_metadata(chinese != 0))) {
        Ok(value) => write_ok_json(out, &value),
        Err(error) => {
            write_error(out, error);
            RT_ERR
        }
    }
}
