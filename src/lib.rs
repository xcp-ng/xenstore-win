//! Xenstore Windows implementation.
//! Rely on xeniface driver.
//!
mod cm;
mod device;
mod ioctl;
mod multiplex;
mod utils;

#[cfg(feature = "smol")]
pub mod smol;
pub mod suspend;

use std::{
    io,
    sync::{Arc, Weak},
};

use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, HANDLE},
        System::Threading::CreateEventW,
    },
    core::{Owned, Result},
};
use xenstore_rs::Xs;

use crate::{
    ioctl::{Xeniface, XenifaceStoreAddWatchOut, XenifaceStoreSuspendRegisterOut},
    multiplex::{MultiplexedXeniface, XenifaceMultiplexWorker},
};

/// Xenstore Windows implementation.
// Note the drop order.
pub struct XsWindows(Arc<XenifaceMultiplexWorker>, Arc<MultiplexedXeniface>);

impl XsWindows {
    pub fn new() -> Result<Self> {
        let iface = MultiplexedXeniface::new();
        let p = Arc::new(iface.start());
        Ok(Self(p, iface))
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self(self.0.clone(), self.1.clone()))
    }

    fn with_device<T>(&self, f: impl FnOnce(&Arc<Xeniface>) -> io::Result<T>) -> io::Result<T> {
        let ptr = self
            .1
            .active()?
            .ok_or(io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND.0 as i32))?;
        f(&ptr)
    }
}

impl Xs for XsWindows {
    fn directory(&self, path: &str) -> io::Result<Vec<Box<str>>> {
        self.with_device(|device| Ok(device.store_directory(path)?))
    }

    fn read(&self, path: &str) -> io::Result<Box<str>> {
        self.with_device(|device| Ok(device.store_read(path)?))
    }

    fn write(&self, path: &str, data: &str) -> io::Result<()> {
        self.with_device(|device| Ok(device.store_write(path, data)?))
    }

    fn rm(&self, path: &str) -> io::Result<()> {
        self.with_device(|device| Ok(device.store_remove(path)?))
    }
}

// Note: watches are bound to their underlying devices and not the active device in XsWindows.
// Therefore, WatchContext will need to embed a reference to its parent device.
pub(crate) struct WatchContext(Weak<Xeniface>, XenifaceStoreAddWatchOut);

impl XsWindows {
    pub(crate) fn make_watch(&self, path: &str) -> io::Result<(Owned<HANDLE>, WatchContext)> {
        let event = unsafe { Owned::new(CreateEventW(None, true, false, None)?) };
        self.with_device(|device| {
            let context = unsafe { device.add_watch(path, *event)? };
            Ok((event, WatchContext(Arc::downgrade(device), context)))
        })
    }

    pub(crate) fn destroy_watch(context: &mut WatchContext) -> io::Result<()> {
        if let Some(ptr) = context.0.upgrade() {
            if ptr.is_active()? {
                ptr.remove_watch(&mut context.1)?;
            }
        }
        Ok(())
    }
}

pub(crate) struct SuspendContext(Weak<Xeniface>, XenifaceStoreSuspendRegisterOut);

impl XsWindows {
    pub(crate) fn make_suspend(&self) -> io::Result<(Owned<HANDLE>, SuspendContext)> {
        let event = unsafe { Owned::new(CreateEventW(None, true, false, None)?) };
        self.with_device(|ptr| {
            let context = unsafe { ptr.suspend_register(*event)? };
            Ok((event, SuspendContext(Arc::downgrade(ptr), context)))
        })
    }

    pub(crate) fn destroy_suspend(context: &mut SuspendContext) -> io::Result<()> {
        if let Some(ptr) = context.0.upgrade() {
            if ptr.is_active()? {
                ptr.suspend_deregister(&mut context.1)?;
            }
        }
        Ok(())
    }
}
