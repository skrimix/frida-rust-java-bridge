use crate::jni;

pub const ACC_PUBLIC: jni::jint = 0x0001;
pub const ACC_PRIVATE: jni::jint = 0x0002;
pub const ACC_PROTECTED: jni::jint = 0x0004;
pub const ACC_STATIC: jni::jint = 0x0008;
pub const ACC_FINAL: jni::jint = 0x0010;
pub const ACC_SYNCHRONIZED: jni::jint = 0x0020;
pub const ACC_BRIDGE: jni::jint = 0x0040;
pub const ACC_VARARGS: jni::jint = 0x0080;
pub const ACC_NATIVE: jni::jint = 0x0100;
pub const ACC_ABSTRACT: jni::jint = 0x0400;
pub const ACC_STRICT: jni::jint = 0x0800;
pub const ACC_SYNTHETIC: jni::jint = 0x1000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_constants_match_java_reflection_flags() {
        assert_eq!(ACC_PUBLIC, 0x0001);
        assert_eq!(ACC_PRIVATE, 0x0002);
        assert_eq!(ACC_PROTECTED, 0x0004);
        assert_eq!(ACC_STATIC, 0x0008);
        assert_eq!(ACC_FINAL, 0x0010);
        assert_eq!(ACC_SYNCHRONIZED, 0x0020);
        assert_eq!(ACC_BRIDGE, 0x0040);
        assert_eq!(ACC_VARARGS, 0x0080);
        assert_eq!(ACC_NATIVE, 0x0100);
        assert_eq!(ACC_ABSTRACT, 0x0400);
        assert_eq!(ACC_STRICT, 0x0800);
        assert_eq!(ACC_SYNTHETIC, 0x1000);
    }
}
