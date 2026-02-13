use std::{
    future::{self, Future},
    io,
    os::windows::io::AsRawHandle,
    pin::Pin,
    task::{Context, Poll},
};

use async_io::os::windows::Waitable;
use event_listener::EventListener;
use futures::{FutureExt, Stream, ready};
use windows::{
    Win32::{Foundation::HANDLE, System::Threading::ResetEvent},
    core::Result,
};
use xenstore_rs::{AsyncWatch, AsyncXs, Xs};

use crate::{
    XsWindows,
    multiplex::{XenifaceSuspend, XenifaceWatch},
    suspend::AsyncSuspend,
    utils::UnsafeBorrowed,
};

pub struct XsSmolWindows(XsWindows);

impl XsSmolWindows {
    pub async fn new() -> Result<Self> {
        Ok(Self(XsWindows::new()?))
    }
}

// TODO: Find a way to use overlapped IO instead.
impl AsyncXs for XsSmolWindows {
    fn directory(&self, path: &str) -> impl Future<Output = io::Result<Vec<Box<str>>>> + Send {
        future::ready(self.0.directory(path))
    }

    fn read(&self, path: &str) -> impl Future<Output = io::Result<Box<str>>> + Send {
        future::ready(self.0.read(path))
    }

    fn write(&self, path: &str, data: &str) -> impl Future<Output = io::Result<()>> + Send {
        future::ready(self.0.write(path, data))
    }

    fn rm(&self, path: &str) -> impl Future<Output = io::Result<()>> + Send {
        future::ready(self.0.rm(path))
    }
}

pub struct XsWindowsWatch {
    waitable: Waitable<UnsafeBorrowed<HANDLE>>,
    _watch: XenifaceWatch,
    path: Box<str>,
}

impl Stream for XsWindowsWatch {
    type Item = Box<str>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Poll::Ready(ready!(self.waitable.poll_ready(cx)).ok().map(|_| {
            unsafe {
                ResetEvent(HANDLE(self.waitable.as_raw_handle()))
                    .inspect_err(|e| log::error!("Unable to reset event handle: {e}"))
                    .ok()
            };
            self.path.clone()
        }))
    }
}

impl AsyncWatch for XsSmolWindows {
    async fn watch(
        &self,
        path: &str,
    ) -> io::Result<impl Stream<Item = Box<str>> + Unpin + 'static> {
        let watch = self.0.make_watch(path)?;
        let handle = watch.get_handle()?;
        let waitable = Waitable::new(unsafe { UnsafeBorrowed::new(handle) })?;

        Ok(XsWindowsWatch {
            waitable,
            _watch: watch,
            path: path.into(),
        })
    }
}

pub struct XsWindowsSuspend {
    waitable: Waitable<UnsafeBorrowed<HANDLE>>,
    _suspend: XenifaceSuspend,
    arrival: EventListener,
}

impl Stream for XsWindowsSuspend {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Ok(_)) = self.waitable.poll_ready(cx) {
            unsafe {
                let _ = ResetEvent(HANDLE(self.waitable.get_ref().as_raw_handle()))
                    .inspect_err(|e| log::error!("Unable to reset event handle: {e}"));
            }
            return Poll::Ready(Some(()));
        }

        if let Poll::Ready(_) = self.arrival.poll_unpin(cx) {
            return Poll::Ready(Some(()));
        }

        Poll::Pending
    }
}

impl AsyncSuspend for XsSmolWindows {
    async fn register_suspend(
        &self,
    ) -> io::Result<impl futures::Stream<Item = ()> + Unpin + 'static> {
        let suspend = self.0.make_suspend()?;
        let handle = suspend.get_handle()?;
        let waitable = Waitable::new(unsafe { UnsafeBorrowed::new(handle) })?;
        let arrival = self.0.iface.listen_arrival()?;

        Ok(XsWindowsSuspend {
            waitable,
            _suspend: suspend,
            arrival,
        })
    }
}
