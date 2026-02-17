use std::{
    ffi::{CString, c_char, c_ulong, c_void},
    sync::{Mutex, MutexGuard, Weak},
};

use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::*,
        Foundation::{ERROR_INVALID_HANDLE, ERROR_NOT_ENOUGH_MEMORY, HANDLE},
        System::{
            IO::DeviceIoControl,
            Ioctl::{FILE_ANY_ACCESS, FILE_DEVICE_UNKNOWN, METHOD_BUFFERED},
        },
    },
    core::GUID,
};

use crate::{
    cm::CmNotifier,
    device::DeviceInfoList,
    multiplex::MultiplexedXeniface,
    utils::{MyOwned, Unwrapped, make_payload, parse_nul_list, parse_nul_string},
};

pub const GUID_INTERFACE_XENIFACE: GUID = GUID::from_values(
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
unsafe impl Send for XenifaceStoreAddWatchOut {}
unsafe impl Sync for XenifaceStoreAddWatchOut {}

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
unsafe impl Send for XenifaceStoreSuspendRegisterOut {}
unsafe impl Sync for XenifaceStoreSuspendRegisterOut {}

const IOCTL_XENIFACE_SUSPEND_REGISTER: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x831, METHOD_BUFFERED, FILE_ANY_ACCESS);

const IOCTL_XENIFACE_SUSPEND_DEREGISTER: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x832, METHOD_BUFFERED, FILE_ANY_ACCESS);

pub(crate) struct Xeniface {
    me: Weak<Xeniface>,
    pub(crate) parent: Weak<MultiplexedXeniface>,
    // The handle must be closed before the notifier
    state: Mutex<Option<(MyOwned<HANDLE>, CmNotifier<Xeniface>)>>,
}

impl Xeniface {
    pub(crate) fn enumerate() -> windows::core::Result<DeviceInfoList> {
        // Try all devices with XENIFACE class.
        DeviceInfoList::new(GUID_INTERFACE_XENIFACE)
    }

    pub(crate) fn new(
        child: &Weak<Self>,
        parent: Weak<MultiplexedXeniface>,
    ) -> windows::core::Result<Self> {
        Ok(Self {
            me: child.clone(),
            parent,
            state: Mutex::new(None),
        })
    }

    pub(crate) fn register(
        &self,
        handle: MyOwned<HANDLE>,
        callback: <PCM_NOTIFY_CALLBACK as Unwrapped>::Inner,
    ) -> windows::core::Result<()> {
        let context = self.me.upgrade().unwrap();

        let mut state = self.lock()?;
        assert!(state.is_none());

        let filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
            u: CM_NOTIFY_FILTER_0 {
                DeviceHandle: CM_NOTIFY_FILTER_0_1 { hTarget: *handle },
            },
            ..Default::default()
        };

        let cm = unsafe { CmNotifier::<Xeniface>::new(&filter, context, callback)? };
        state.replace((handle, cm));
        Ok(())
    }

    fn lock(
        &self,
    ) -> windows::core::Result<MutexGuard<'_, Option<(MyOwned<HANDLE>, CmNotifier<Xeniface>)>>>
    {
        let state = self
            .state
            .lock()
            .map_err(|_| windows::core::Error::from(ERROR_INVALID_HANDLE))?;
        Ok(state)
    }

    pub fn is_active(&self) -> windows::core::Result<bool> {
        let state = self.lock()?;
        Ok(state.is_some())
    }

    pub fn close(&self) -> windows::core::Result<()> {
        let mut state = self.lock()?;
        if let Some(state) = state.as_mut() {
            drop(std::mem::replace(&mut state.0, Default::default()));
        }
        Ok(())
    }

    unsafe fn raw_ioctl(
        &self,
        control_code: u32,
        in_buffer: &[u8],
        out_buffer: Option<&mut [u8]>,
    ) -> windows::core::Result<u32> {
        let mut len = 0;
        let out_buffer_len = out_buffer.as_deref().map_or(0, |s| s.len()) as u32;

        let lock = self.lock()?;
        let handle = lock
            .as_ref()
            .ok_or(windows::core::Error::from(ERROR_INVALID_HANDLE))?;

        unsafe {
            DeviceIoControl(
                *(*handle).0,
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

    unsafe fn ioctl<In, Out>(
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

        let len = unsafe {
            self.raw_ioctl(
                IOCTL_XENIFACE_STORE_DIRECTORY,
                &in_buffer,
                Some(&mut out_buffer),
            )?
        };
        out_buffer.truncate(len as usize);

        Ok(parse_nul_list(&out_buffer)
            .iter()
            .map(|s| String::from_utf8_lossy(*s).into_owned().into_boxed_str())
            .collect())
    }

    pub fn store_read(&self, path: &str) -> windows::core::Result<Box<str>> {
        let in_buffer = make_payload(&[path]);
        let mut out_buffer = vec![0u8; 4096];

        let len = unsafe {
            self.raw_ioctl(IOCTL_XENIFACE_STORE_READ, &in_buffer, Some(&mut out_buffer))?
        };
        out_buffer.truncate(len as usize);

        Ok(
            String::from_utf8_lossy(parse_nul_string(&out_buffer).unwrap_or_default())
                .into_owned()
                .into_boxed_str(),
        )
    }

    pub fn store_write(&self, path: &str, data: &str) -> windows::core::Result<()> {
        let in_buffer = make_payload(&[path, data]);

        unsafe {
            self.raw_ioctl(IOCTL_XENIFACE_STORE_WRITE, &in_buffer, None)?;
        }

        Ok(())
    }

    pub fn store_remove(&self, path: &str) -> windows::core::Result<()> {
        let in_buffer = make_payload(&[path]);

        unsafe {
            self.raw_ioctl(IOCTL_XENIFACE_STORE_REMOVE, &in_buffer, None)?;
        }

        Ok(())
    }

    pub unsafe fn add_watch<'a>(
        &'a self,
        path: &str,
        event: HANDLE,
    ) -> windows::core::Result<XenifaceStoreAddWatchOut> {
        let c_path = CString::new(path)
            .map_err(|_| windows::core::Error::from_hresult(ERROR_NOT_ENOUGH_MEMORY.into()))?;
        let path_bytes = c_path.to_bytes_with_nul();

        let watch_in = XenifaceStoreAddWatchIn {
            path: path_bytes.as_ptr() as *const c_char,
            path_length: path_bytes.len() as u32,
            event: event,
        };
        let mut context = XenifaceStoreAddWatchOut {
            context: std::ptr::null_mut(),
        };

        unsafe {
            self.ioctl(
                IOCTL_XENIFACE_STORE_ADD_WATCH,
                &watch_in,
                Some(&mut context),
            )?;
        }

        Ok(context)
    }

    pub fn remove_watch(
        &self,
        context: &mut XenifaceStoreAddWatchOut,
    ) -> windows::core::Result<()> {
        unsafe {
            self.ioctl::<XenifaceStoreAddWatchOut, c_void>(
                IOCTL_XENIFACE_STORE_REMOVE_WATCH,
                context,
                None,
            )?;
        }
        Ok(())
    }

    pub unsafe fn suspend_register<'a>(
        &'a self,
        event: HANDLE,
    ) -> windows::core::Result<XenifaceStoreSuspendRegisterOut> {
        let suspend_in = XenifaceStoreSuspendRegisterIn { event: event };
        let mut context = XenifaceStoreSuspendRegisterOut {
            context: std::ptr::null_mut(),
        };

        unsafe {
            self.ioctl(
                IOCTL_XENIFACE_SUSPEND_REGISTER,
                &suspend_in,
                Some(&mut context.context),
            )?;
        }

        Ok(context)
    }

    pub fn suspend_deregister(
        &self,
        context: &mut XenifaceStoreSuspendRegisterOut,
    ) -> windows::core::Result<()> {
        unsafe {
            self.ioctl::<XenifaceStoreSuspendRegisterOut, c_void>(
                IOCTL_XENIFACE_SUSPEND_DEREGISTER,
                context,
                None,
            )?;
        }
        Ok(())
    }
}
