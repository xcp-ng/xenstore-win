use std::ffi::{CString, c_char, c_ulong, c_void};

use log::{debug, warn};
use windows::{
    Win32::{
        Foundation::{
            DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_FOUND,
            GENERIC_READ, GENERIC_WRITE, HANDLE,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{FILE_ANY_ACCESS, FILE_DEVICE_UNKNOWN, METHOD_BUFFERED},
            Threading::GetCurrentProcess,
        },
    },
    core::{GUID, Owned, PCWSTR},
};

use crate::{
    device::DeviceInfoList,
    utils::{make_payload, parse_nul_list, parse_nul_string},
};

const GUID_INTERFACE_XENIFACE: GUID = GUID::from_values(
    0xb2cfb085,
    0xaa5e,
    0x47e1,
    [0x8b, 0xf7, 0x97, 0x93, 0xf3, 0x15, 0x45, 0x65],
);

// Well, there is no CTL_CODE in the windows crate so we need to add it ourselves.
// Taken from https://docs.rs/winapi/latest/src/winapi/um/winioctl.rs.html#146-153
const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

const IOCTL_XENIFACE_STORE_READ: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_STORE_WRITE: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_STORE_DIRECTORY: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_STORE_REMOVE: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x803, METHOD_BUFFERED, FILE_ANY_ACCESS);

#[repr(C)]
struct XenifaceStoreAddWatchIn {
    path: *const c_char,
    path_length: c_ulong,
    event: HANDLE,
}

#[repr(C)]
pub(crate) struct XenifaceStoreAddWatchOut {
    context: *const c_void,
}

const IOCTL_XENIFACE_STORE_ADD_WATCH: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x805, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_STORE_REMOVE_WATCH: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x806, METHOD_BUFFERED, FILE_ANY_ACCESS);

#[repr(C)]
struct XenifaceStoreSuspendRegisterIn {
    event: HANDLE,
}

#[repr(C)]
pub(crate) struct XenifaceStoreSuspendRegisterOut {
    context: *const c_void,
}

const IOCTL_XENIFACE_SUSPEND_REGISTER: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x831, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_SUSPEND_DEREGISTER: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x832, METHOD_BUFFERED, FILE_ANY_ACCESS);

pub(crate) struct Xeniface(Owned<HANDLE>);

impl Xeniface {
    pub fn new() -> windows::core::Result<Self> {
        // Try all devices with XENIFACE class.
        let dev_list = DeviceInfoList::new(GUID_INTERFACE_XENIFACE).unwrap();

        for raw_wpath in dev_list.iter() {
            let wpath = PCWSTR::from_raw(raw_wpath.as_ptr());
            debug!("Trying {}", unsafe { wpath.display() });

            match unsafe {
                CreateFileW(
                    wpath,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES::default(),
                    None,
                )
            } {
                Ok(file) => {
                    debug!("Got {file:?}");
                    return Ok(Self(unsafe { Owned::new(file) }));
                }
                Err(e) => {
                    warn!("Unable to open {} ({e})", unsafe { wpath.display() })
                }
            }
        }

        return Err(ERROR_NOT_FOUND.into());
    }

    pub fn try_clone(&self) -> windows::core::Result<Self> {
        unsafe {
            let mut new_handle = HANDLE::default();
            DuplicateHandle(
                GetCurrentProcess(),
                *self.0,
                GetCurrentProcess(),
                &mut new_handle,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )?;
            Ok(Self(Owned::new(new_handle)))
        }
    }

    fn raw_ioctl(
        &self,
        control_code: u32,
        in_buffer: &[u8],
        out_buffer: Option<&mut [u8]>,
    ) -> windows::core::Result<u32> {
        let mut len = 0;
        let out_buffer_len = out_buffer.as_deref().map_or(0, size_of_val) as u32;

        unsafe {
            DeviceIoControl(
                *self.0,
                control_code,
                Some(in_buffer.as_ptr().cast()),
                in_buffer.len() as u32,
                out_buffer.map(|r| r.as_mut_ptr().cast()),
                out_buffer_len,
                Some(&mut len),
                None,
            )?;
        }

        Ok(len)
    }

    fn ioctl<In, Out>(
        &self,
        control_code: u32,
        in_val: &In,
        out_val: Option<&mut Out>,
    ) -> windows::core::Result<u32> {
        unsafe {
            let in_slice =
                core::slice::from_raw_parts(in_val as *const In as *const u8, size_of_val(in_val));
            let out_slice = out_val
                .map(|r| core::slice::from_raw_parts_mut(r as *mut Out as *mut u8, size_of_val(r)));
            self.raw_ioctl(control_code, in_slice, out_slice)
        }
    }

    pub fn store_directory(&self, path: &str) -> windows::core::Result<Vec<Box<str>>> {
        let in_buffer = make_payload(&[path]);
        let mut out_buffer = vec![0u8; 4096];

        let len = self.raw_ioctl(
            IOCTL_XENIFACE_STORE_DIRECTORY,
            &in_buffer,
            Some(&mut out_buffer),
        )?;
        out_buffer.truncate(len as usize);

        Ok(parse_nul_list(&out_buffer)
            .iter()
            .map(|s| String::from_utf8_lossy(*s).into_owned().into_boxed_str())
            .collect())
    }

    pub fn store_read(&self, path: &str) -> windows::core::Result<Box<str>> {
        let in_buffer = make_payload(&[path]);
        let mut out_buffer = vec![0u8; 4096];

        let len = self.raw_ioctl(IOCTL_XENIFACE_STORE_READ, &in_buffer, Some(&mut out_buffer))?;
        out_buffer.truncate(len as usize);

        Ok(
            String::from_utf8_lossy(parse_nul_string(&out_buffer).unwrap_or_default())
                .into_owned()
                .into_boxed_str(),
        )
    }

    pub fn store_write(&self, path: &str, data: &str) -> windows::core::Result<()> {
        let in_buffer = make_payload(&[path, data]);

        self.raw_ioctl(IOCTL_XENIFACE_STORE_WRITE, &in_buffer, None)?;

        Ok(())
    }

    pub fn store_remove(&self, path: &str) -> windows::core::Result<()> {
        let in_buffer = make_payload(&[path]);

        self.raw_ioctl(IOCTL_XENIFACE_STORE_REMOVE, &in_buffer, None)?;

        Ok(())
    }

    pub fn add_watch<'a>(
        &'a self,
        path: &str,
        event: HANDLE,
    ) -> windows::core::Result<XenifaceStoreAddWatchOut> {
        let c_path = CString::new(path)
            .map_err(|_| windows::core::Error::from_hresult(ERROR_NOT_ENOUGH_MEMORY.into()))?;

        let watch_in = XenifaceStoreAddWatchIn {
            path: c_path.as_ptr(),
            path_length: (c_path.count_bytes() + 1) as u32,
            event: event,
        };
        let mut context = XenifaceStoreAddWatchOut {
            context: std::ptr::null_mut(),
        };

        self.ioctl(
            IOCTL_XENIFACE_STORE_ADD_WATCH,
            &watch_in,
            Some(&mut context),
        )?;

        Ok(context)
    }

    pub fn remove_watch(
        &self,
        context: &mut XenifaceStoreAddWatchOut,
    ) -> windows::core::Result<()> {
        self.ioctl::<XenifaceStoreAddWatchOut, c_void>(
            IOCTL_XENIFACE_STORE_REMOVE_WATCH,
            context,
            None,
        )?;
        Ok(())
    }

    pub fn suspend_register<'a>(
        &'a self,
        event: HANDLE,
    ) -> windows::core::Result<XenifaceStoreSuspendRegisterOut> {
        let suspend_in = XenifaceStoreSuspendRegisterIn { event: event };
        let mut context = XenifaceStoreSuspendRegisterOut {
            context: std::ptr::null_mut(),
        };

        self.ioctl(
            IOCTL_XENIFACE_SUSPEND_REGISTER,
            &suspend_in,
            Some(&mut context.context),
        )?;

        Ok(context)
    }

    pub fn suspend_deregister(
        &self,
        context: &mut XenifaceStoreSuspendRegisterOut,
    ) -> windows::core::Result<()> {
        self.ioctl::<XenifaceStoreSuspendRegisterOut, c_void>(
            IOCTL_XENIFACE_SUSPEND_DEREGISTER,
            context,
            None,
        )?;
        Ok(())
    }
}
