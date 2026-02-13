use std::{ffi::c_void, ops::DerefMut, sync::Arc};

use windows::{
    Win32::{Devices::DeviceAndDriverInstallation::*, Foundation::ERROR_GEN_FAILURE},
    core::HRESULT,
};

use crate::utils::{MyOwned, Unwrapped};

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

pub(crate) struct CmNotifier<T> {
    _listener: MyOwned<HCMNOTIFICATION>,
    _ptr: DerefedArcPtr<T>,
}
unsafe impl<T> Send for CmNotifier<T> {}

impl<T> CmNotifier<T> {
    pub unsafe fn new(
        filter: &CM_NOTIFY_FILTER,
        context: Arc<T>,
        callback: <PCM_NOTIFY_CALLBACK as Unwrapped>::Inner,
    ) -> windows::core::Result<Self> {
        let mut listener = MyOwned::<HCMNOTIFICATION>::default();
        let ptr = DerefedArcPtr::new(context);
        unsafe {
            match CM_Register_Notification(
                filter,
                Some(ptr.0 as *const c_void),
                Some(callback),
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
