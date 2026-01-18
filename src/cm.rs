use std::{ffi::c_void, ops::DerefMut, sync::Arc};

use windows::{
    Win32::{Devices::DeviceAndDriverInstallation::*, Foundation::ERROR_GEN_FAILURE},
    core::{HRESULT, Owned},
};

struct DerefedArcPtr<T>(*const T);

impl<T> DerefedArcPtr<T> {
    fn new(ptr: Arc<T>) -> Self {
        Self(Arc::into_raw(ptr))
    }
}

impl<T> Drop for DerefedArcPtr<T> {
    fn drop(&mut self) {
        drop(unsafe { Arc::from_raw(self.0) });
    }
}

pub(crate) struct CmNotifier<T: Send + Sync> {
    _listener: Owned<HCMNOTIFICATION>,
    _ptr: DerefedArcPtr<T>,
}
unsafe impl<T: Send + Sync> Send for CmNotifier<T> {}
unsafe impl<T: Send + Sync> Sync for CmNotifier<T> {}

impl<T: Send + Sync> CmNotifier<T> {
    pub fn new(
        filter: &CM_NOTIFY_FILTER,
        context: Arc<T>,
        callback: PCM_NOTIFY_CALLBACK,
    ) -> windows::core::Result<Self> {
        let mut listener = Owned::<HCMNOTIFICATION>::default();
        let ptr = DerefedArcPtr::new(context);
        unsafe {
            match CM_Register_Notification(
                filter,
                Some(ptr.0 as *const c_void),
                callback,
                listener.deref_mut(),
            ) {
                CR_SUCCESS => Ok(Self {
                    _listener: listener,
                    _ptr: ptr,
                }),
                e => {
                    let err = CM_MapCrToWin32Err(e, ERROR_GEN_FAILURE.0);
                    Err(HRESULT::from_win32(err).into())
                }
            }
        }
    }
}
