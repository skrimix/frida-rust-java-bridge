use std::{
    ffi::{c_int, c_void},
    fs,
    ptr::{self, NonNull},
};

use super::{
    features::FEATURE_METHOD_QUERY,
    layout::{normalize_address, unsupported_method_query},
};
use crate::error::{Error, Result};

#[derive(Debug, Default)]
pub(super) struct MemoryRanges {
    pub(super) ranges: Vec<MemoryRange>,
}

#[derive(Debug)]
pub(super) struct MemoryRange {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) writable: bool,
    pub(super) executable: bool,
}

pub(super) struct ExecutableMemory {
    pub(super) pointer: NonNull<c_void>,
    length: usize,
}

unsafe impl Send for ExecutableMemory {}
unsafe impl Sync for ExecutableMemory {}

impl ExecutableMemory {
    #[cfg(target_arch = "aarch64")]
    pub(super) fn aarch64_pretty_method_thunk(target: *const c_void) -> Result<Self> {
        let _gum = crate::native::process_gum();
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        let length = 32;
        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                length,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_method_query("unable to allocate PrettyMethod ABI thunk");
        }

        let mut code = [0u8; 32];
        write_u32_le(&mut code, 0, 0xaa0003e8); // mov x8, x0
        write_u32_le(&mut code, 4, 0xaa0103e0); // mov x0, x1
        write_u32_le(&mut code, 8, 0xaa0203e1); // mov x1, x2
        write_u32_le(&mut code, 12, 0x58000047); // ldr x7, #8
        write_u32_le(&mut code, 16, 0xd61f00e0); // br x7
        write_u64_le(&mut code, 20, target as usize as u64);

        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), pointer.cast::<u8>(), code.len());
            frida_gum_sys::gum_clear_cache(pointer, length as u64);
            if mprotect(pointer, length, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, length);
                return unsupported_method_query("unable to protect PrettyMethod ABI thunk");
            }
        }

        let pointer = NonNull::new(pointer).ok_or(Error::NullReturn { operation: "mmap" })?;
        Ok(Self { pointer, length })
    }
}

impl Drop for ExecutableMemory {
    fn drop(&mut self) {
        unsafe {
            munmap(self.pointer.as_ptr(), self.length);
        }
    }
}

impl MemoryRanges {
    pub(super) fn current() -> Result<Self> {
        Self::current_for_feature(FEATURE_METHOD_QUERY)
    }

    pub(super) fn current_for_feature(feature: &'static str) -> Result<Self> {
        let maps =
            fs::read_to_string("/proc/self/maps").map_err(|error| Error::UnsupportedFeature {
                feature,
                reason: format!("unable to read /proc/self/maps: {error}"),
            })?;
        let mut ranges = Vec::new();
        for line in maps.lines() {
            let mut columns = line.split_whitespace();
            let Some(addresses) = columns.next() else {
                continue;
            };
            let Some(perms) = columns.next() else {
                continue;
            };
            if !perms.starts_with('r') {
                continue;
            }
            let Some((start, end)) = addresses.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end)) = (
                usize::from_str_radix(start, 16),
                usize::from_str_radix(end, 16),
            ) else {
                continue;
            };
            ranges.push(MemoryRange {
                start,
                end,
                writable: perms.as_bytes().get(1) == Some(&b'w'),
                executable: perms.as_bytes().get(2) == Some(&b'x'),
            });
        }
        Ok(Self { ranges })
    }

    pub(super) fn contains(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|range| {
            let range_start = normalize_address(range.start);
            let range_end = normalize_address(range.end);
            address >= range_start && end <= range_end
        })
    }

    pub(super) fn contains_executable(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|range| {
            let range_start = normalize_address(range.start);
            let range_end = normalize_address(range.end);
            range.executable && address >= range_start && end <= range_end
        })
    }

    pub(super) fn contains_writable(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|range| {
            let range_start = normalize_address(range.start);
            let range_end = normalize_address(range.end);
            range.writable && address >= range_start && end <= range_end
        })
    }
}

pub(super) fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn write_u64_le(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

unsafe extern "C" {
    pub(super) fn mmap(
        address: *mut c_void,
        length: usize,
        protection: c_int,
        flags: c_int,
        file_descriptor: c_int,
        offset: isize,
    ) -> *mut c_void;
    pub(super) fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
    pub(super) fn munmap(address: *mut c_void, length: usize) -> c_int;
}
