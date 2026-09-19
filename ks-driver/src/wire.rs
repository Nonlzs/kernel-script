//! Fixed-layout views for the METHOD_BUFFERED driver ABI.
//!
//! These types are intentionally local to the driver. The shared protocol
//! crate remains dependency-free, while zerocopy makes validation and byte
//! casting at this trust boundary explicit.

use zerocopy::{FromBytes, Immutable, KnownLayout};

#[repr(C)]
#[derive(FromBytes, Immutable, KnownLayout)]
pub struct MemoryReadRequest {
    pub process_id: u64,
    pub address: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(FromBytes, Immutable, KnownLayout)]
pub struct MemoryWriteRequest {
    pub process_id: u64,
    pub address: u64,
    pub size: u64,
    pub data: [u8; 4096],
}

#[repr(C)]
#[derive(FromBytes, Immutable, KnownLayout)]
pub struct ProcessBaseRequest {
    pub process_id: u64,
}

#[repr(C)]
#[derive(FromBytes, Immutable, KnownLayout)]
pub struct MemoryRvaReadRequest {
    pub process_id: u64,
    pub relative_address: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(FromBytes, Immutable, KnownLayout)]
pub struct MemoryRvaWriteRequest {
    pub process_id: u64,
    pub relative_address: u64,
    pub size: u64,
    pub data: [u8; 4096],
}
