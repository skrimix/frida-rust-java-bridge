use std::ffi::{CStr, c_char, c_void};

use crate::error::{Error, Result};

#[repr(C)]
pub(super) struct ArtStdString {
    pub(super) storage: [usize; 3],
}

impl ArtStdString {
    pub(super) fn to_string(&self) -> Result<String> {
        let data = self.data();
        if data.is_null() {
            return Err(Error::NullReturn {
                operation: "std::string::c_str",
            });
        }
        unsafe { CStr::from_ptr(data) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from)
    }

    pub(super) fn data(&self) -> *const c_char {
        if self.storage[0] & 1 != 0 {
            self.storage[2] as *const c_char
        } else {
            (self as *const Self).cast::<u8>().wrapping_add(1).cast()
        }
    }

    pub(super) fn destroy(&mut self) {
        if self.storage[0] & 1 != 0 {
            unsafe { free(self.storage[2] as *mut c_void) };
        }
    }
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}
