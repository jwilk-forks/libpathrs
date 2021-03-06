/*
 * libpathrs: safe path resolution on Linux
 * Copyright (C) 2019-2020 Aleksa Sarai <cyphar@cyphar.com>
 * Copyright (C) 2019-2020 SUSE LLC
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU Lesser General Public License as published by the Free
 * Software Foundation, either version 3 of the License, or (at your option) any
 * later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT ANY
 * WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
 * PARTICULAR PURPOSE. See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public License along
 * with this program. If not, see <https://www.gnu.org/licenses/>.
 */

use crate::{
    capi::utils::{CHandle, CPointerType, CRoot, ErrorWrap, Leakable},
    error::{self, Error},
    utils::RawFdExt,
    Handle, Root,
};

use std::{
    os::unix::io::{IntoRawFd, RawFd},
    ptr,
};

use libc::c_void;

/// Duplicate a file-based libpathrs object.
///
/// The new object will have a separate lifetime from the original, but will
/// refer to the same underlying file (and contain the same configuration, if
/// applicable).
///
/// Only certain objects can be duplicated with pathrs_duplicate():
///
///   * PATHRS_ROOT, with pathrs_root_t.
///   * PATHRS_HANDLE, with pathrs_handle_t.
///
/// If an error occurs, NULL is returned. The object passed with this request
/// will store the error (which can be retrieved with pathrs_error). If the
/// object type is not one of the permitted values above, the error is lost.
#[no_mangle]
pub extern "C" fn pathrs_duplicate(ptr_type: CPointerType, ptr: *const c_void) -> *mut c_void {
    if ptr.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: All of these casts and dereferences are safe because the C caller
    //         has assured us that the type passed is correct. We also make sure
    //         to not create aliased &muts by accident by destructuring the
    //         CPointer<T>s into (inner, last_error).
    match ptr_type {
        CPointerType::PATHRS_NONE | CPointerType::PATHRS_ERROR => ptr::null_mut(),
        CPointerType::PATHRS_ROOT => {
            // SAFETY: See above.
            let root = unsafe { &*(ptr as *const CRoot) };
            root.wrap_err(ptr::null_mut(), |root| {
                root.try_clone()
                    .map(CRoot::from)
                    .map(Leakable::leak)
                    .map(|p| p as *mut _ as *mut c_void)
            })
        }
        CPointerType::PATHRS_HANDLE => {
            // SAFETY: See above.
            let handle = unsafe { &*(ptr as *const CHandle) };
            handle.wrap_err(ptr::null_mut(), |handle| {
                handle
                    .try_clone()
                    .map(CHandle::from)
                    .map(Leakable::leak)
                    .map(|p| p as *mut _ as *mut c_void)
            })
        }
        _ => panic!("invalid ptr_type: {:?}", ptr_type),
    }
}

/// Unwrap a file-based libpathrs object to obtain its underlying file
/// descriptor.
///
/// The main purpose of this interface (combined with pathrs_from_fd) is to
/// allow for libpathrs objects to be passed to other processes through Unix
/// sockets (with SCM_RIGHTS) or other such tricks. The underlying file
/// descriptor of such an object can be thought of as the "serialised" version
/// of the object.
///
/// This consumes the original object, and it is the caller's responsibility to
/// close the file descriptor (with close) or otherwise handle its lifetime.
///
/// Only certain objects can be converted into file descriptors with
/// pathrs_into_fd():
///
///   * PATHRS_ROOT, with pathrs_root_t.
///   * PATHRS_HANDLE, with pathrs_handle_t.
///
/// It is critical that you do not operate on this file descriptor yourself,
/// because the security properties of libpathrs depend on users doing all
/// relevant filesystem operations through libpathrs.
///
/// If an error occurs, -1 is returned. You may retrieve the error by calling
/// pathrs_error on the passed object (as long as the object is one of the
/// permitted ones listed above).
///
/// If an error occurs, -1 is returned. The object passed with this request will
/// store the error (which can be retrieved with pathrs_error). If the object
/// type is not one of the permitted values above, the error is lost.
#[no_mangle]
pub extern "C" fn pathrs_into_fd(ptr_type: CPointerType, ptr: *const c_void) -> RawFd {
    if ptr.is_null() {
        return -1;
    }

    // SAFETY: All of these casts and dereferences are safe because the C caller
    //         has assured us that the type passed is correct. We also make sure
    //         to not create aliased &muts by accident by destructuring the
    //         CPointer<T>s into (inner, last_error).
    match ptr_type {
        CPointerType::PATHRS_NONE | CPointerType::PATHRS_ERROR => -1,
        CPointerType::PATHRS_ROOT => {
            // SAFETY: See above.
            let root = unsafe { &*(ptr as *const CRoot) };
            root.take_wrap_err(-1, |root| Ok(root.into_file().into_raw_fd()))
        }
        CPointerType::PATHRS_HANDLE => {
            // SAFETY: See above.
            let handle = unsafe { &*(ptr as *const CHandle) };
            handle.take_wrap_err(-1, |handle| Ok(handle.into_file().into_raw_fd()))
        }
        _ => panic!("invalid ptr_type: {:?}", ptr_type),
    }
}

/// Construct a new file-based libpathrs object from a file descriptor.
///
/// The main purpose of this interface (combined with pathrs_into_fd) is to
/// allow for libpathrs objects to be passed to other processes through Unix
/// sockets (with SCM_RIGHTS) or other such tricks. The underlying file
/// descriptor of such an object can be thought of as the "serialised" version
/// of the object, and this method effectively "de-serialises" it.
///
/// Note that libpathrs will duplicate the file descriptor passed to it (in
/// order to avoid higher-level language runtimes from accidentally closing the
/// file descriptor). The caller must therefore close the file descriptor passed
/// if they no longer require it after this call.
///
/// Only certain objects can be constructed from file descriptors with
/// pathrs_from_fd():
///
///   * PATHRS_ROOT, producing a pathrs_root_t.
///     (NOTE: The configuration will be the system default.)
///   * PATHRS_HANDLE, producing a pathrs_handle_t.
///
/// It is critical that the file descriptor provided has the same semantics as
/// file descriptors which libpathrs would generate itself. This usually means
/// that you should only ever call pathrs_from_fd() with a file descriptor that
/// originally came from pathrs_into_fd().
///
/// If an error occurs, an object of the requested type is returned containing
/// the error (which can be retrieved with pathrs_error) -- as with pathrs_open.
/// If the object type requested is not one of the permitted values above, NULL
/// is returned.
#[no_mangle]
pub extern "C" fn pathrs_from_fd(fd_type: CPointerType, fd: RawFd) -> *mut c_void {
    let mut last_error: Option<Error> = None;
    let ret = last_error.wrap(ptr::null_mut(), move || {
        ensure!(
            fd >= 0,
            error::InvalidArgument {
                name: "fd",
                description: "negative fd value",
            }
        );

        // Make a copy of the file. It's entirely possible that some language
        // runtimes (or programs) will not be able to uphold the contract that
        // the file ownerships is now ours. So it's much less of a headache to
        // just duplicate the handle so that the caller never sees the fd we
        // actually end up using.
        let file = fd.try_clone_hotfix()?;

        match fd_type {
            CPointerType::PATHRS_ROOT => {
                // SAFETY: The C caller guarantees this file is a valid Root.
                let root: CRoot = Root::from_file_unchecked(file).into();
                // Leak and switch to void pointer.
                Ok(root.leak() as *mut _ as *mut c_void)
            }
            CPointerType::PATHRS_HANDLE => {
                // SAFETY: The C caller guarantees this file is a valid Handle.
                let handle: CHandle = Handle::from_file_unchecked(file).into();
                // Leak and switch to void pointer.
                Ok(handle.leak() as *mut _ as *mut c_void)
            }
            _ => error::InvalidArgument {
                name: "fd_type",
                description: "invalid pathrs type to construct from fd",
            }
            .fail(),
        }
    });

    // If there was an error, we construct a new object with the requested type
    // (if we can) so that the caller can get a proper error through
    // pathrs_error(). Unfortunately this is a bit ugly.
    match last_error {
        None => ret,
        Some(err) => match fd_type {
            CPointerType::PATHRS_ROOT => CRoot::from_err(err).leak() as *mut _ as *mut c_void,
            CPointerType::PATHRS_HANDLE => CHandle::from_err(err).leak() as *mut _ as *mut c_void,
            // Nothing more we can do. We could return a CError for
            // PATHRS_ERROR, but callers might not correctly handle that (if you
            // call pathrs_error(PATHRS_ERROR) you currently get NULL).
            CPointerType::PATHRS_NONE | CPointerType::PATHRS_ERROR => ptr::null_mut(),
            _ => panic!("invalid fd_type: {:?}", fd_type),
        },
    }
}
