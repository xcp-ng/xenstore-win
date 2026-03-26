use std::{
    ffi::c_void,
    ops::ControlFlow,
    sync::{Arc, Mutex, MutexGuard, Weak},
    thread::JoinHandle,
};

use event_listener::{Event, EventListener};
use flume::{Receiver, Sender, TryRecvError};
use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::*,
        Foundation::*,
        Storage::FileSystem::*,
        System::Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects},
    },
    core::PCWSTR,
};

use crate::{
    cm::CmNotifier,
    device::{
        GUID_INTERFACE_XENIFACE, Xeniface, XenifaceStoreAddWatchOut,
        XenifaceStoreSuspendRegisterOut,
    },
    utils::{MyOwned, UnsafeBorrowed},
};

pub(crate) enum XenifaceRequest {
    Worker(CM_NOTIFY_ACTION),
    Listener {
        action: CM_NOTIFY_ACTION,
        target: Arc<Xeniface>,
    },
}

// Note: watches are bound to their underlying devices and not the active device in XsWindows.
// Therefore, MultiplexedWatchNodeState will need to embed a reference to its parent device.

struct WatchHandle(Weak<Xeniface>, XenifaceStoreAddWatchOut);

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(ptr) = self.0.upgrade() {
            if let Ok(true) = ptr.is_active() {
                let _ = ptr
                    .remove_watch(&mut self.1)
                    .inspect_err(|e| log::error!("Failed to remove watch: {e}"));
            }
        }
    }
}

struct WatchNodeState {
    watch: Option<WatchHandle>,
    event: MyOwned<HANDLE>,
    path: String,
}

struct WatchNode
where
    Self: Send,
{
    link: LinkedListAtomicLink,
    state: Mutex<WatchNodeState>,
}

intrusive_adapter!(WatchAdapter = Arc<WatchNode>: WatchNode { link => LinkedListAtomicLink });

struct SuspendHandle(Weak<Xeniface>, XenifaceStoreSuspendRegisterOut);

impl Drop for SuspendHandle {
    fn drop(&mut self) {
        if let Some(ptr) = self.0.upgrade() {
            if let Ok(true) = ptr.is_active() {
                let _ = ptr
                    .suspend_deregister(&mut self.1)
                    .inspect_err(|e| log::error!("Failed to remove suspend: {e}"));
            }
        }
    }
}

struct SuspendNodeState {
    suspend: Option<SuspendHandle>,
    event: MyOwned<HANDLE>,
}

struct SuspendNode
where
    Self: Send,
{
    link: LinkedListAtomicLink,
    state: Mutex<SuspendNodeState>,
}

intrusive_adapter!(SuspendAdapter = Arc<SuspendNode>: SuspendNode { link => LinkedListAtomicLink });

struct MultiplexState {
    worker: Option<JoinHandle<windows::core::Result<()>>>,
    active: Option<Arc<Xeniface>>,
    sender: Option<Sender<XenifaceRequest>>,
    arrival: Event,
    watches: LinkedList<WatchAdapter>,
    suspends: LinkedList<SuspendAdapter>,
}

pub(crate) struct MultiplexedXeniface
where
    Self: Send + Sync,
{
    me: Weak<MultiplexedXeniface>,
    recv_event: MyOwned<HANDLE>,
    state: Mutex<MultiplexState>,
}

pub struct XenifaceWatch(Arc<MultiplexedXeniface>, Arc<WatchNode>);

impl XenifaceWatch {
    pub fn get_handle(&self) -> windows::core::Result<HANDLE> {
        let node_lock = self
            .1
            .state
            .lock()
            .map_err(|_| windows::core::Error::from(ERROR_INVALID_HANDLE))?;
        Ok(*node_lock.event)
    }
}

impl Drop for XenifaceWatch {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            if let Ok(mut node_lock) = self.1.state.lock() {
                drop(node_lock.watch.take());
            }
            unsafe {
                drop(state.watches.cursor_mut_from_ptr(self.1.as_ref()).remove());
            }
        }
    }
}

pub struct XenifaceSuspend(Arc<MultiplexedXeniface>, Arc<SuspendNode>);

impl XenifaceSuspend {
    pub fn get_handle(&self) -> windows::core::Result<HANDLE> {
        let node_lock = self
            .1
            .state
            .lock()
            .map_err(|_| windows::core::Error::from(ERROR_INVALID_HANDLE))?;
        Ok(*node_lock.event)
    }

    pub fn listen_arrival(&self) -> windows::core::Result<EventListener> {
        self.0.listen_arrival()
    }
}

impl Drop for XenifaceSuspend {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            if let Ok(mut node_lock) = self.1.state.lock() {
                drop(node_lock.suspend.take());
            }
            unsafe {
                drop(state.suspends.cursor_mut_from_ptr(self.1.as_ref()).remove());
            }
        }
    }
}

pub struct XenifaceWorker(Weak<MultiplexedXeniface>)
where
    Self: Send;

impl Drop for XenifaceWorker {
    fn drop(&mut self) {
        if let Some(x) = self.0.upgrade() {
            x.stop();
        };
    }
}

impl MultiplexedXeniface {
    pub fn new() -> windows::core::Result<Arc<Self>> {
        let recv_event = unsafe { MyOwned::new(CreateEventW(None, true, false, None)?) };
        let result = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            recv_event,
            state: Mutex::new(MultiplexState {
                worker: None,
                active: None,
                sender: None,
                arrival: Event::new(),
                watches: Default::default(),
                suspends: Default::default(),
            }),
        });
        Ok(result)
    }

    pub fn start(&self) -> XenifaceWorker {
        let mut state = self.state.lock().unwrap();
        assert!(state.worker.is_none());
        assert!(state.sender.is_none());

        // for worker Cm notify messages
        let (sender, receiver) = flume::unbounded();
        // initial probe
        sender
            .send(XenifaceRequest::Worker(
                CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
            ))
            .unwrap();
        unsafe { SetEvent(*self.recv_event).unwrap() };
        let ptr = self.me.upgrade().clone().unwrap();

        let recv_event = unsafe { UnsafeBorrowed::new(*self.recv_event) };
        state.sender = Some(sender);
        state.worker = Some(std::thread::spawn(move || {
            Self::worker(ptr, receiver, recv_event)
        }));
        XenifaceWorker(self.me.clone())
    }

    pub(crate) fn stop(&self) {
        log::info!("Stopping worker");
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

    fn lock(&self) -> windows::core::Result<MutexGuard<'_, MultiplexState>> {
        self.state
            .lock()
            .map_err(|_| windows::core::Error::from(ERROR_INVALID_HANDLE))
    }

    pub fn active(&self) -> windows::core::Result<Option<Arc<Xeniface>>> {
        let state = self.lock()?;
        Ok(state.active.clone())
    }

    pub fn listen_arrival(&self) -> windows::core::Result<EventListener> {
        let state = self.lock()?;
        Ok(state.arrival.listen())
    }

    pub fn add_watch(&self, path: &str) -> windows::core::Result<XenifaceWatch> {
        let mut state = self.lock()?;

        let event = unsafe { MyOwned::new(CreateEventW(None, true, false, None)?) };
        let handle = *event;
        let node = Arc::new(WatchNode {
            link: Default::default(),
            state: Mutex::new(WatchNodeState {
                watch: None,
                event,
                path: String::from(path),
            }),
        });
        if let Some(active) = state.active.as_ref() {
            let mut node_lock = node.state.lock().unwrap();
            let watch_out = unsafe { active.add_watch(path, handle)? };
            node_lock
                .watch
                .replace(WatchHandle(Arc::downgrade(active), watch_out));
        }

        state.watches.push_back(node.clone());
        Ok(XenifaceWatch(self.me.upgrade().unwrap(), node))
    }

    pub fn register_suspend(&self) -> windows::core::Result<XenifaceSuspend> {
        let mut state = self.lock()?;

        let event = unsafe { MyOwned::new(CreateEventW(None, true, false, None)?) };
        let handle = *event;
        let node = Arc::new(SuspendNode {
            link: Default::default(),
            state: Mutex::new(SuspendNodeState {
                suspend: None,
                event,
            }),
        });
        if let Some(active) = state.active.as_ref() {
            let mut node_lock = node.state.lock().unwrap();
            let suspend_out = unsafe { active.suspend_register(handle)? };
            node_lock
                .suspend
                .replace(SuspendHandle(Arc::downgrade(active), suspend_out));
        }

        state.suspends.push_back(node.clone());
        Ok(XenifaceSuspend(self.me.upgrade().unwrap(), node))
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
                if let Ok(_) = sender.send(XenifaceRequest::Worker(action)) {
                    unsafe {
                        let _ = SetEvent(*this.recv_event)
                            .inspect_err(|e| log::error!("Cannot signal recv event: {e}"));
                    };
                }
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
                let new = Arc::into_raw(arc);
                assert!(new == context as *const Xeniface);
                child
            };

            if action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVE
                || action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED
            {
                log::debug!("CM_NOTIFY_ACTION_DEVICEQUERYREMOVE/FAILED");
                let _ = child
                    .close()
                    .inspect_err(|e| log::error!("Cannot close interface: {e}"));
            }

            let parent = child.parent.upgrade().ok_or(ERROR_NOT_FOUND.0)?;
            let sender = {
                let state = parent.state.lock().map_err(|_| ERROR_INVALID_HANDLE.0)?;
                state.sender.clone()
            };
            if let Some(sender) = sender {
                if let Ok(_) = sender.send(XenifaceRequest::Listener {
                    action,
                    target: child,
                }) {
                    unsafe {
                        let _ = SetEvent(*parent.recv_event)
                            .inspect_err(|e| log::error!("Cannot signal recv event: {e}"));
                    };
                }
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
            MyOwned::new(CreateFileW(
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
            Xeniface::new(child, self.me.clone()).unwrap()
        });
        result.register(handle, Self::listener_callback)?;
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
                    log::warn!("Unable to open {} ({})", unsafe { wpath.display() }, e);
                }
            }
        }

        Err(ERROR_NOT_FOUND.into())
    }

    fn rearm_watches(watches: &mut LinkedList<WatchAdapter>, next: &Arc<Xeniface>) {
        log::debug!("Rearming watches");
        watches.iter().for_each(|w| {
            if let Ok(mut node_lock) = w.state.lock() {
                match unsafe { next.add_watch(&node_lock.path, *node_lock.event) } {
                    Ok(watch_out) => {
                        node_lock
                            .watch
                            .replace(WatchHandle(Arc::downgrade(next), watch_out));
                    }
                    Err(e) => {
                        // stale?
                        log::error!("Failed to rearm watch: {e}");
                        node_lock.watch.take();
                    }
                }
            }
        });
    }

    fn rearm_suspends(suspends: &mut LinkedList<SuspendAdapter>, next: &Arc<Xeniface>) {
        log::debug!("Rearming suspends");
        suspends.iter().for_each(|s| {
            if let Ok(mut node_lock) = s.state.lock() {
                match unsafe { next.suspend_register(*node_lock.event) } {
                    Ok(suspend_out) => {
                        node_lock
                            .suspend
                            .replace(SuspendHandle(Arc::downgrade(&next), suspend_out));
                    }
                    Err(e) => {
                        // stale?
                        log::error!("Failed to rearm suspend: {e}");
                        node_lock.suspend.take();
                    }
                }
            }
        });
    }

    fn refresh(
        &self,
        state: &mut MutexGuard<'_, MultiplexState>,
        tombstones: &mut Vec<Arc<Xeniface>>,
    ) -> windows::core::Result<()> {
        log::info!("Refreshing");

        if let Some(active) = state.active.as_ref() {
            if active
                .is_active()
                .inspect_err(|e| log::error!("Cannot inspect active: {e}"))?
            {
                log::info!("Device valid, skipping refresh");
                return Ok(());
            } else {
                log::info!("Tombstoning existing inactive device");
                tombstones.push(state.active.take().unwrap());
            }
        }

        let paths: Vec<Box<[u16]>> = Xeniface::enumerate()
            .inspect_err(|e| log::error!("Enumerate failed: {e}"))?
            .iter()
            .collect();

        if paths.is_empty() {
            log::info!("Interface list empty");
            return Err(ERROR_NOT_FOUND.into());
        }

        let next = self.open(&paths)?;
        state.active = Some(next.clone());

        Self::rearm_watches(&mut state.watches, &next);
        Self::rearm_suspends(&mut state.suspends, &next);
        state.arrival.notify(usize::MAX);

        Ok(())
    }

    fn worker_do_recv(
        &self,
        receiver: &Receiver<XenifaceRequest>,
        tombstones: &mut Vec<Arc<Xeniface>>,
    ) -> ControlFlow<()> {
        let mut state = self.state.lock().unwrap();
        loop {
            match receiver.try_recv() {
                Ok(XenifaceRequest::Worker(CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL)) => {
                    log::info!("CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL");
                    if let Err(e) = self.refresh(&mut state, tombstones) {
                        log::info!("Refresh failed: {e}")
                    }
                }
                Ok(XenifaceRequest::Listener { action, target }) => {
                    if action == CM_NOTIFY_ACTION_DEVICEREMOVEPENDING
                        || action == CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE
                    {
                        log::info!("CM_NOTIFY_ACTION_DEVICEREMOVEPENDING/COMPLETE");
                        state.active.take_if(|active| Arc::ptr_eq(&target, active));
                        tombstones.push(target);
                    }
                }
                Err(TryRecvError::Disconnected) => return ControlFlow::Break(()),
                Err(TryRecvError::Empty) => return ControlFlow::Continue(()),
                _ => (),
            }
        }
    }

    fn worker(
        self: Arc<Self>,
        receiver: Receiver<XenifaceRequest>,
        recv_event: UnsafeBorrowed<HANDLE>,
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

        let _cr = unsafe {
            CmNotifier::<Self>::new(
                &filter,
                self.me.upgrade().unwrap().clone(),
                Self::worker_cm_callback,
            )?
        };

        let suspend = self.register_suspend()?;
        let events = [*recv_event, suspend.get_handle()?];
        loop {
            match unsafe { WaitForMultipleObjects(&events, false, INFINITE) } {
                WAIT_EVENT(0) => {
                    log::debug!("Worker got request event");
                    unsafe {
                        let _ = ResetEvent(events[0])
                            .inspect_err(|e| log::error!("Cannot reset request event: {e}"));
                    };
                    if let ControlFlow::Break(_) = self.worker_do_recv(&receiver, &mut tombstones) {
                        break;
                    }
                }
                WAIT_EVENT(1) => {
                    log::debug!("Worker got suspend event");
                    unsafe {
                        let _ = ResetEvent(events[1])
                            .inspect_err(|e| log::error!("Cannot reset suspend event: {e}"));
                    };
                    let state = &mut *self.state.lock().unwrap();
                    if let Some(active) = state.active.as_mut() {
                        Self::rearm_watches(&mut state.watches, active);
                    }
                }
                WAIT_FAILED => panic!("{}", windows::core::Error::from_thread()),
                _ => unreachable!(),
            }

            tombstones.clear();
        }

        log::info!("Worker exiting");
        Ok(())
    }
}
