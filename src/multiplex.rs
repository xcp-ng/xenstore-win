use std::{
    ffi::c_void,
    sync::{Arc, Mutex, MutexGuard, Weak, mpsc},
    thread::JoinHandle,
};

use windows::{
    Win32::{Devices::DeviceAndDriverInstallation::*, Foundation::*, Storage::FileSystem::*},
    core::{Owned, PCWSTR},
};

use crate::{
    cm::CmNotifier,
    ioctl::{GUID_INTERFACE_XENIFACE, Xeniface},
};

pub(crate) enum XenifaceRequest {
    Worker(CM_NOTIFY_ACTION),
    Listener {
        action: CM_NOTIFY_ACTION,
        target: Arc<Xeniface>,
    },
}

struct MultiplexState {
    worker: Option<JoinHandle<windows::core::Result<()>>>,
    active: Option<Arc<Xeniface>>,
    sender: Option<mpsc::Sender<XenifaceRequest>>,
}

pub(crate) struct MultiplexedXeniface {
    me: Weak<MultiplexedXeniface>,
    state: Mutex<MultiplexState>,
}

pub struct XenifaceMultiplexWorker(Weak<MultiplexedXeniface>);

impl Drop for XenifaceMultiplexWorker {
    fn drop(&mut self) {
        if let Some(x) = self.0.upgrade() {
            x.stop();
        };
    }
}

impl MultiplexedXeniface {
    pub fn new() -> Arc<Self> {
        let result = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            state: Mutex::new(MultiplexState {
                worker: None,
                active: None,
                sender: None,
            }),
        });
        result
    }

    pub fn start(&self) -> XenifaceMultiplexWorker {
        let mut state = self.state.lock().unwrap();
        assert!(state.worker.is_none());
        assert!(state.sender.is_none());
        let (sender, receiver) = mpsc::channel();
        let ptr = self.me.upgrade().clone().unwrap();
        state.sender = Some(sender);
        state.worker = Some(std::thread::spawn(move || Self::worker(ptr, receiver)));
        XenifaceMultiplexWorker(self.me.clone())
    }

    pub(crate) fn stop(&self) {
        let (sender, mut worker) = {
            let mut state = self.state.lock().unwrap();
            (state.sender.take(), state.worker.take())
        };
        drop(sender);
        if let Some(t) = worker.take() {
            let _ = t
                .join()
                .inspect_err(|_| log::error!("Worker thread failed"));
        }
    }

    pub fn active(&self) -> windows::core::Result<Option<Arc<Xeniface>>> {
        let state = self
            .state
            .lock()
            .map_err(|_| windows::core::Error::from(ERROR_INVALID_HANDLE))?;
        Ok(state.active.clone())
    }

    unsafe extern "system" fn worker_cm_callback(
        _hnotify: HCMNOTIFICATION,
        context: *const c_void,
        action: CM_NOTIFY_ACTION,
        _eventdata: *const CM_NOTIFY_EVENT_DATA,
        eventdatasize: u32,
    ) -> u32 {
        let result = (|| {
            if context.is_null() {
                return Err(ERROR_NOT_FOUND.0);
            }
            if (eventdatasize as usize) < size_of::<CM_NOTIFY_EVENT_DATA>() {
                return Err(ERROR_ASSERTION_FAILURE.0);
            }

            let this = unsafe { &*(context as *const Self) };
            let sender = {
                let state = this.state.lock().map_err(|_| ERROR_INVALID_HANDLE.0)?;
                state.sender.clone()
            };
            if let Some(sender) = sender {
                let _ = sender.send(XenifaceRequest::Worker(action));
            }

            Ok(())
        })();
        match result {
            Ok(()) => ERROR_SUCCESS.0,
            Err(e) => e,
        }
    }

    unsafe extern "system" fn listener_callback(
        _hnotify: HCMNOTIFICATION,
        context: *const c_void,
        action: CM_NOTIFY_ACTION,
        _eventdata: *const CM_NOTIFY_EVENT_DATA,
        _eventdatasize: u32,
    ) -> u32 {
        let result = (|| {
            if context.is_null() {
                return Err(ERROR_NOT_FOUND.0);
            }

            let child = unsafe {
                let arc = Arc::from_raw(context as *const Xeniface);
                let child = arc.clone();
                let _ = Arc::into_raw(arc);
                child
            };

            if action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVE
                || action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED
            {
                log::debug!("CM_NOTIFY_ACTION_DEVICEQUERYREMOVE/FAILED");
                let mut iface = child.lock().map_err(|_| ERROR_INVALID_HANDLE.0)?;
                iface.take();
            }

            let parent = child.parent.upgrade().ok_or(ERROR_NOT_FOUND.0)?;
            let sender = {
                let state = parent.state.lock().map_err(|_| ERROR_INVALID_HANDLE.0)?;
                state.sender.clone()
            };
            if let Some(sender) = sender {
                let _ = sender.send(XenifaceRequest::Listener {
                    action,
                    target: child,
                });
            }

            Ok(())
        })();
        match result {
            Ok(()) => ERROR_SUCCESS.0,
            Err(e) => e,
        }
    }

    fn open_raw(&self, wpath: PCWSTR) -> windows::core::Result<Arc<Xeniface>> {
        let handle = unsafe {
            Owned::new(CreateFileW(
                wpath,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES::default(),
                None,
            )?)
        };
        let result = Arc::new_cyclic(|child: &Weak<Xeniface>| {
            let result = Xeniface::new(child, self.me.clone()).unwrap();
            result
        });
        result.register(handle, Some(Self::listener_callback))?;
        Ok(result)
    }

    fn open(&self, paths: &Vec<Box<[u16]>>) -> windows::core::Result<Arc<Xeniface>> {
        for raw_wpath in paths {
            let wpath = PCWSTR::from_raw(raw_wpath.as_ptr());
            log::debug!("Trying {}", unsafe { wpath.display() });

            match self.open_raw(wpath) {
                Ok(xeniface) => {
                    return Ok(xeniface);
                }
                Err(e) => {
                    log::warn!("Unable to open {} ({e})", unsafe { wpath.display() })
                }
            }
        }

        Err(ERROR_NOT_FOUND.into())
    }

    fn refresh(
        &self,
        state: &mut MutexGuard<'_, MultiplexState>,
        tombstones: &mut Vec<Arc<Xeniface>>,
    ) -> windows::core::Result<()> {
        let paths: Vec<Box<[u16]>> = Xeniface::enumerate()?.iter().collect();

        if paths.is_empty() {
            log::debug!("Interface list empty");
            return Err(ERROR_NOT_FOUND.into());
        }

        if let Some(active) = state.active.as_ref() {
            if active.is_active()? {
                log::debug!("Device valid, skipping refresh");
                return Ok(());
            }
        }

        if let Some(active) = state.active.take() {
            tombstones.push(active);
        }

        let next = self.open(&paths)?;
        state.active = Some(next);

        Ok(())
    }

    fn worker(
        self: Arc<Self>,
        receiver: mpsc::Receiver<XenifaceRequest>,
    ) -> windows::core::Result<()> {
        let mut tombstones = Vec::<Arc<Xeniface>>::new();

        let filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                    ClassGuid: GUID_INTERFACE_XENIFACE,
                },
            },
            ..Default::default()
        };

        let _cr = CmNotifier::<Self>::new(
            &filter,
            self.me.upgrade().unwrap().clone(),
            Some(Self::worker_cm_callback),
        )?;

        {
            let mut state = self.state.lock().unwrap();
            if let Err(e) = self.refresh(&mut state, &mut tombstones) {
                log::info!("Refresh failed: {e}")
            }
        }

        while let Ok(request) = receiver.recv() {
            {
                let mut state = self.state.lock().unwrap();
                match request {
                    XenifaceRequest::Worker(CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL) => {
                        if let Err(e) = self.refresh(&mut state, &mut tombstones) {
                            log::info!("Refresh failed: {e}")
                        }
                    }
                    XenifaceRequest::Listener {
                        action:
                            CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE,
                        target,
                    } => {
                        if let Some(active) = state.active.as_ref() {
                            if Arc::ptr_eq(&target, active) {
                                state.active = None
                            }
                        }
                        tombstones.push(target);
                    }
                    _ => {}
                }
            }

            tombstones.clear();
        }

        Ok(())
    }
}
