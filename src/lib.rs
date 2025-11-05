//! Xenstore Windows implementation.
//! Rely on xeniface driver.
//!
mod device;
mod ioctl;
mod utils;

#[cfg(feature = "smol")]
pub mod smol;
pub mod suspend;

use std::io;

use windows::{
    Win32::{Foundation::HANDLE, System::Threading::CreateEventW},
    core::{Owned, Result},
};
use xenstore_rs::Xs;

use crate::ioctl::{Xeniface, XenifaceStoreAddWatchOut, XenifaceStoreSuspendRegisterOut};

/// Xenstore Windows implementation.
pub struct XsWindows(Xeniface);

impl XsWindows {
    /// Try to open Xenstore interface.
    ///
    /// Uses the first working xeniface device.
    pub fn new() -> Result<Self> {
        let xeniface = Xeniface::new()?;
        Ok(Self(xeniface))
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self(self.0.try_clone()?))
    }
}

impl Xs for XsWindows {
    fn directory(&self, path: &str) -> io::Result<Vec<Box<str>>> {
        Ok(self.0.store_directory(path)?)
    }

    fn read(&self, path: &str) -> io::Result<Box<str>> {
        Ok(self.0.store_read(path)?)
    }

    fn write(&self, path: &str, data: &str) -> io::Result<()> {
        Ok(self.0.store_write(path, data)?)
    }

    fn rm(&self, path: &str) -> io::Result<()> {
        Ok(self.0.store_remove(path)?)
    }
}

pub(crate) struct WatchContext(XenifaceStoreAddWatchOut);
unsafe impl Send for WatchContext {}

impl XsWindows {
    pub(crate) fn make_watch(&self, path: &str) -> io::Result<(Owned<HANDLE>, WatchContext)> {
        let event = unsafe { Owned::new(CreateEventW(None, true, false, None)?) };
        let context = self.0.add_watch(path, *event)?;
        Ok((event, WatchContext(context)))
    }

    pub(crate) fn destroy_watch(&self, context: &mut WatchContext) -> io::Result<()> {
        self.0.remove_watch(&mut context.0)?;
        Ok(())
    }
}

pub(crate) struct SuspendContext(XenifaceStoreSuspendRegisterOut);
unsafe impl Send for SuspendContext {}

impl XsWindows {
    pub(crate) fn make_suspend(&self) -> io::Result<(Owned<HANDLE>, SuspendContext)> {
        let event = unsafe { Owned::new(CreateEventW(None, true, false, None)?) };
        let context = self.0.suspend_register(*event)?;
        Ok((event, SuspendContext(context)))
    }

    pub(crate) fn destroy_suspend(&self, context: &mut SuspendContext) -> io::Result<()> {
        self.0.suspend_deregister(&mut context.0)?;
        Ok(())
    }
}

unsafe impl Send for XsWindows {}
unsafe impl Sync for XsWindows {}
