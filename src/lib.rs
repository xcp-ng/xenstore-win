//! Xenstore Windows implementation.
//! Rely on xeniface driver.
//!
mod cm;
mod device;
mod devicelist;
mod multiplex;
mod utils;

#[cfg(feature = "smol")]
pub mod smol;
pub mod suspend;

use std::{
    io,
    sync::{Arc, LazyLock},
};

use windows::{Win32::Foundation::ERROR_FILE_NOT_FOUND, core::Result};
use xenstore_rs::Xs;

use crate::{
    device::Xeniface,
    multiplex::{MultiplexedXeniface, XenifaceSuspend, XenifaceWatch, XenifaceWorker},
};

/// Xenstore Windows implementation.
// Note the drop order.
pub struct XsWindows {
    worker: Arc<XenifaceWorker>,
    iface: Arc<MultiplexedXeniface>,
}

static XENIFACE: LazyLock<XsWindows> =
    LazyLock::new(|| XsWindows::new_instance().expect("Failed to start Xeniface multiplexer"));

impl XsWindows {
    fn new_instance() -> Result<Self> {
        let iface = MultiplexedXeniface::new()?;
        let worker = Arc::new(iface.start());
        Ok(XsWindows { worker, iface })
    }

    pub fn new() -> Result<Self> {
        Ok(Self {
            worker: XENIFACE.worker.clone(),
            iface: XENIFACE.iface.clone(),
        })
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            worker: self.worker.clone(),
            iface: self.iface.clone(),
        })
    }

    fn with_device<T>(&self, f: impl FnOnce(&Xeniface) -> io::Result<T>) -> io::Result<T> {
        let ptr = self
            .iface
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

impl XsWindows {
    pub(crate) fn make_watch(&self, path: &str) -> io::Result<XenifaceWatch> {
        let watch = self.iface.add_watch(path)?;
        Ok(watch)
    }

    pub(crate) fn make_suspend(&self) -> io::Result<XenifaceSuspend> {
        let suspend = self.iface.register_suspend()?;
        Ok(suspend)
    }
}
