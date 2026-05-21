use super::*;

impl Env<'_> {
    pub fn find_class(&self, name: &str) -> Result<ClassRef<'_>> {
        let class = self.find_class_raw(name)?;
        unsafe { LocalRef::from_raw(self, class) }
    }

    pub fn find_class_raw(&self, name: &str) -> Result<jni::jclass> {
        let name = CString::new(name)?;
        let find_class = self.function::<jni::FindClass>(jni::ENV_FIND_CLASS);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let class = unsafe { find_class(self.handle.as_ptr(), name.as_ptr()) };
        self.check_pending_exception("JNIEnv::FindClass")?;

        if class.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::FindClass",
            })
        } else {
            Ok(class)
        }
    }

    pub fn new_string_utf(&self, text: &str) -> Result<StringRef<'_>> {
        let string = self.new_string_utf_raw(text)?;
        unsafe { LocalRef::from_raw(self, string) }
    }

    pub fn new_string_utf_raw(&self, text: &str) -> Result<jni::jstring> {
        let text = CString::new(text)?;
        let new_string_utf = self.function::<jni::NewStringUtf>(jni::ENV_NEW_STRING_UTF);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let string = unsafe { new_string_utf(self.handle.as_ptr(), text.as_ptr()) };
        self.check_pending_exception("JNIEnv::NewStringUTF")?;

        if string.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewStringUTF",
            })
        } else {
            Ok(string)
        }
    }

    pub fn get_string(&self, string: &StringRef<'_>) -> Result<String> {
        unsafe { self.get_string_raw(string.raw_jstring()) }
    }

    pub fn get_string_utf(&self, string: &StringRef<'_>) -> Result<String> {
        unsafe { self.get_string_utf_raw(string.raw_jstring()) }
    }

    /// Copies a Java string into a Rust `String` through JNI's UTF-16 string accessors.
    ///
    /// # Safety
    ///
    /// `string` must be a valid `jstring` local or global reference for this VM.
    pub unsafe fn get_string_raw(&self, string: jni::jstring) -> Result<String> {
        let get_string_length = self.function::<jni::GetStringLength>(jni::ENV_GET_STRING_LENGTH);
        let get_string_chars = self.function::<jni::GetStringChars>(jni::ENV_GET_STRING_CHARS);
        let release_string_chars =
            self.function::<jni::ReleaseStringChars>(jni::ENV_RELEASE_STRING_CHARS);
        let mut is_copy = jni::JNI_FALSE;

        let length = unsafe { get_string_length(self.handle.as_ptr(), string) };
        let chars = unsafe { get_string_chars(self.handle.as_ptr(), string, &mut is_copy) };
        if chars.is_null() {
            self.check_pending_exception("JNIEnv::GetStringChars")?;
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetStringChars",
            });
        }

        let chars = unsafe { std::slice::from_raw_parts(chars, length as usize) };
        let result =
            char::decode_utf16(chars.iter().copied()).collect::<std::result::Result<String, _>>();

        unsafe { release_string_chars(self.handle.as_ptr(), string, chars.as_ptr()) };

        result.map_err(Error::from)
    }

    /// Copies a Java string into a Rust `String`.
    ///
    /// # Safety
    ///
    /// `string` must be a valid `jstring` local or global reference for this VM.
    pub unsafe fn get_string_utf_raw(&self, string: jni::jstring) -> Result<String> {
        let get_string_utf_chars =
            self.function::<jni::GetStringUtfChars>(jni::ENV_GET_STRING_UTF_CHARS);
        let release_string_utf_chars =
            self.function::<jni::ReleaseStringUtfChars>(jni::ENV_RELEASE_STRING_UTF_CHARS);
        let mut is_copy = jni::JNI_FALSE;

        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and `string`
        // is expected to be a valid jstring owned by the caller.
        let chars = unsafe { get_string_utf_chars(self.handle.as_ptr(), string, &mut is_copy) };
        if chars.is_null() {
            self.check_pending_exception("JNIEnv::GetStringUTFChars")?;
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetStringUTFChars",
            });
        }

        // SAFETY: JNI returned a non-null, NUL-terminated modified UTF-8 buffer.
        let result = unsafe { CStr::from_ptr(chars) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from);

        // SAFETY: The buffer came from GetStringUTFChars for the same jstring/env pair.
        unsafe { release_string_utf_chars(self.handle.as_ptr(), string, chars) };

        result
    }
}
