//! Raw IOKit bindings and the `AppleSMC` user-client wire format.
//!
//! The struct layout mirrors the kernel's `SMCParamStruct` (80 bytes). It is
//! undocumented but has been stable from Intel Macs through Apple Silicon.

#![allow(non_camel_case_types, non_upper_case_globals)]

use std::os::raw::{c_char, c_void};

pub type kern_return_t = i32;
pub type mach_port_t = u32;
pub type io_object_t = mach_port_t;
pub type io_service_t = io_object_t;
pub type io_connect_t = io_object_t;

pub const KERN_SUCCESS: kern_return_t = 0;
pub const kIOMainPortDefault: mach_port_t = 0;

/// `IOConnectCallStructMethod` selector used by AppleSMC for all key traffic.
pub const KERNEL_INDEX_SMC: u32 = 2;

pub const SMC_CMD_READ_BYTES: u8 = 5;
pub const SMC_CMD_WRITE_BYTES: u8 = 6;
pub const SMC_CMD_READ_INDEX: u8 = 8;
pub const SMC_CMD_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SMCVersion {
    pub major: u8,
    pub minor: u8,
    pub build: u8,
    pub reserved: u8,
    pub release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SMCPLimitData {
    pub version: u16,
    pub length: u16,
    pub cpu_p_limit: u32,
    pub gpu_p_limit: u32,
    pub mem_p_limit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SMCKeyInfoData {
    pub data_size: u32,
    pub data_type: u32,
    pub data_attributes: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SMCParamStruct {
    pub key: u32,
    pub vers: SMCVersion,
    pub p_limit_data: SMCPLimitData,
    pub key_info: SMCKeyInfoData,
    pub result: u8,
    pub status: u8,
    pub data8: u8,
    pub data32: u32,
    pub bytes: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<SMCParamStruct>() == 80);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    pub fn IOServiceGetMatchingService(main_port: mach_port_t, matching: *mut c_void) -> io_service_t;
    pub fn IOServiceOpen(
        service: io_service_t,
        owning_task: mach_port_t,
        kind: u32,
        connect: *mut io_connect_t,
    ) -> kern_return_t;
    pub fn IOServiceClose(connect: io_connect_t) -> kern_return_t;
    pub fn IOObjectRelease(object: io_object_t) -> kern_return_t;
    pub fn IOConnectCallStructMethod(
        connection: io_connect_t,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> kern_return_t;
}

extern "C" {
    pub static mach_task_self_: mach_port_t;
}
