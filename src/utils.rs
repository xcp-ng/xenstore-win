/// Some NUL-string payload related utilities.
/// Taken from xenstore-rs wire.rs
///
use std::{
    io::Write,
    ops::{Deref, DerefMut},
    os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, RawHandle},
    str::{self},
};

use windows::{Win32::Foundation::HANDLE, core::Free};

pub fn make_payload(strings: &[&str]) -> Box<[u8]> {
    let mut payload: Vec<u8> = Vec::new();

    for s in strings {
        payload.write_all(s.as_bytes()).unwrap(); // infailble
        payload.push(0);
    }
    if strings.len() > 1 {
        payload.push(0);
    }

    payload.into_boxed_slice()
}

pub fn parse_nul_string(mut buffer: &[u8]) -> Option<&[u8]> {
    // Assuming terminating NUL
    if buffer.is_empty() {
        None
    } else {
        // Discard latest NUL character (if present)
        if buffer.last() == Some(&0) {
            buffer = &buffer[..buffer.len() - 1];
        }

        Some(buffer)
    }
}

pub fn parse_nul_list(buffer: &[u8]) -> Box<[&[u8]]> {
    buffer
        .split_inclusive(|&c| c == 0)
        .filter_map(|s| parse_nul_string(s))
        .collect()
}

pub(crate) trait Unwrapped {
    type Inner;
}

impl<T> Unwrapped for Option<T> {
    type Inner = T;
}

impl<T, E> Unwrapped for Result<T, E> {
    type Inner = T;
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Default, Debug)]
pub struct MyOwned<T: Free>(T);

impl<T: Free> MyOwned<T> {
    pub unsafe fn new(x: T) -> Self {
        Self(x)
    }
}

impl<T: Free> Drop for MyOwned<T> {
    fn drop(&mut self) {
        unsafe { self.0.free() };
    }
}

impl<T: Free> Deref for MyOwned<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Free> DerefMut for MyOwned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl Send for MyOwned<HANDLE> {}
unsafe impl Sync for MyOwned<HANDLE> {}

pub(crate) struct UnsafeBorrowed<T>(T);

impl<T> UnsafeBorrowed<T> {
    pub(crate) unsafe fn new(value: T) -> Self {
        Self(value)
    }
}

impl AsHandle for UnsafeBorrowed<HANDLE> {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        unsafe { BorrowedHandle::borrow_raw(self.0.0) }
    }
}

impl AsRawHandle for UnsafeBorrowed<HANDLE> {
    fn as_raw_handle(&self) -> RawHandle {
        self.0.0
    }
}

impl<T: Free> Deref for UnsafeBorrowed<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Free> DerefMut for UnsafeBorrowed<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl Send for UnsafeBorrowed<HANDLE> {}
unsafe impl Sync for UnsafeBorrowed<HANDLE> {}
