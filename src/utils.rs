/// Some NUL-string payload related utilities.
/// Taken from xenstore-rs wire.rs
///
use std::{
    io::Write,
    str::{self},
};

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
