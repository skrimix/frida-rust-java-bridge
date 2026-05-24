#[cfg(test)]
mod tests {
    use std::{
        ffi::c_void,
        ptr::{self, NonNull},
        sync::atomic::Ordering,
    };

    use super::super::{
        backend::*,
        enumeration::{
            ArtClassLoaderVisitor, ArtClassVisitor, PrettyMethodFunction, object_class_reference,
        },
        features::*,
        layout::*,
        replacement::*,
        support::*,
    };
    use crate::{env::MethodKind, error::Error, jni, runtime::FeatureSupport, vm::Vm};

    const QUICK_RESOLUTION_TEST_STUB: usize = 0x1000_0000;
    const QUICK_IMT_CONFLICT_TEST_STUB: usize = 0x1000_1000;
    const QUICK_GENERIC_JNI_TEST_STUB: usize = 0x1000_2000;
    const QUICK_TO_INTERPRETER_TEST_STUB: usize = 0x1000_3000;

    unsafe extern "C" fn dummy_add_global_ref(
        _vm: *mut jni::JavaVM,
        _thread: *mut c_void,
        _object: *mut c_void,
    ) -> jni::jobject {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn dummy_suspend_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_resume_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_visit_class_loaders(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassLoaderVisitor,
    ) {
    }

    unsafe extern "C" fn dummy_visit_classes(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassVisitor,
    ) {
    }

    unsafe extern "C" fn dummy_visit_objects(
        _heap: *mut c_void,
        _callback: HeapObjectCallback,
        _context: *mut c_void,
    ) {
    }

    unsafe extern "C" fn dummy_decode_global(
        _vm: *mut jni::JavaVM,
        _object: jni::jobject,
    ) -> usize {
        0x1234
    }

    unsafe extern "C" fn dummy_pretty_method(
        _result: *mut ArtStdString,
        _method: *mut c_void,
        _with_signature: bool,
    ) {
    }

    unsafe extern "C" fn dummy_decode_method_id(
        _jni_id_manager: *mut c_void,
        _method_id: jni::jmethodID,
    ) -> *mut c_void {
        0x1234usize as *mut c_void
    }

    unsafe extern "C" fn dummy_is_quick_resolution_stub(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_RESOLUTION_TEST_STUB
    }

    unsafe extern "C" fn dummy_is_quick_to_interpreter_bridge(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_TO_INTERPRETER_TEST_STUB
    }

    unsafe extern "C" fn dummy_is_quick_generic_jni_stub(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_GENERIC_JNI_TEST_STUB
    }

    fn dummy_entrypoint_predicates() -> ArtClassLinkerEntrypointPredicates {
        ArtClassLinkerEntrypointPredicates {
            is_quick_resolution_stub: dummy_is_quick_resolution_stub,
            is_quick_to_interpreter_bridge: dummy_is_quick_to_interpreter_bridge,
            is_quick_generic_jni_stub: dummy_is_quick_generic_jni_stub,
        }
    }

    #[test]
    fn derives_api_26_runtime_offsets() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(26, vm_offset),
            vec![vm_offset - STD_STRING_SIZE - (2 * POINTER_SIZE)]
        );
    }

    #[test]
    fn derives_api_30_runtime_offset_candidates() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(30, vm_offset),
            vec![
                vm_offset - (3 * POINTER_SIZE),
                vm_offset - (4 * POINTER_SIZE)
            ]
        );
    }

    #[test]
    fn derives_heap_offset_from_thread_list_offset() {
        let thread_list_offset = 512;
        assert_eq!(
            heap_offset_for_api(30, thread_list_offset),
            thread_list_offset - (8 * POINTER_SIZE)
        );
        assert_eq!(
            heap_offset_for_api(34, thread_list_offset),
            thread_list_offset - (9 * POINTER_SIZE)
        );
    }

    #[test]
    fn detects_runtime_layout_from_supported_offsets() {
        let vm_offset = 512;
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];
        let vm_value = 0x1234usize;
        let heap = 0x1500usize as *mut c_void;
        let thread_list = 0x2000usize as *mut c_void;
        let class_linker = 0x3000usize as *mut c_void;
        let intern_table = 0x3500usize as *mut c_void;
        let jni_id_manager = 0x4000usize as *mut c_void;

        runtime[vm_offset / POINTER_SIZE] = vm_value;
        runtime[(vm_offset - POINTER_SIZE) / POINTER_SIZE] = jni_id_manager as usize;
        runtime[(vm_offset - (13 * POINTER_SIZE)) / POINTER_SIZE] = heap as usize;
        runtime[(vm_offset - (5 * POINTER_SIZE)) / POINTER_SIZE] = thread_list as usize;
        runtime[(vm_offset - (4 * POINTER_SIZE)) / POINTER_SIZE] = intern_table as usize;
        runtime[(vm_offset - (3 * POINTER_SIZE)) / POINTER_SIZE] = class_linker as usize;

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                vm_value,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Ok(ArtRuntimeLayout {
                runtime: runtime.as_mut_ptr().cast(),
                heap,
                thread_list,
                class_linker,
                intern_table,
                jni_id_manager,
                jni_ids_indirection: None,
            })
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn tagged_custom_ranges_match_normalized_addresses() {
        let base = 0x0012_3456_0000usize;
        let tag = 0xab00_0000_0000_0000usize;
        let tagged_start = base | tag;
        let tagged_end = (base + 0x1000) | tag;
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: tagged_start,
                end: tagged_end,
                executable: true,
            }],
        };
        let module = ArtModuleRange {
            start: tagged_start,
            end: tagged_end,
        };

        assert!(memory.contains(base + 0x100, POINTER_SIZE));
        assert!(memory.contains_executable(base + 0x100, POINTER_SIZE));
        assert!(module.contains(base + 0x100));
    }

    #[test]
    fn replacement_runtime_layout_rejects_invalid_class_linker_candidate() {
        let vm_offset = 512;
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];
        let mut invalid_class_linker = vec![0u8; 320];
        let mut valid_class_linker = vec![0u8; 320];
        let mut code = vec![0u8; 96];
        let vm_value = 0x1234usize;
        let heap = 0x1500usize as *mut c_void;
        let thread_list = 0x2000usize as *mut c_void;
        let intern_table = 0x3500usize as *mut c_void;
        let quick_resolution = code.as_mut_ptr() as usize;
        let quick_imt_conflict = unsafe { code.as_mut_ptr().add(16) as usize };
        let quick_generic_jni = unsafe { code.as_mut_ptr().add(32) as usize };
        let quick_to_interpreter = unsafe { code.as_mut_ptr().add(48) as usize };
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);

        runtime[vm_offset / POINTER_SIZE] = vm_value;
        runtime[(vm_offset - (14 * POINTER_SIZE)) / POINTER_SIZE] = heap as usize;
        runtime[(vm_offset - (6 * POINTER_SIZE)) / POINTER_SIZE] = thread_list as usize;
        runtime[(vm_offset - (5 * POINTER_SIZE)) / POINTER_SIZE] = intern_table as usize;
        runtime[(vm_offset - (4 * POINTER_SIZE)) / POINTER_SIZE] =
            valid_class_linker.as_mut_ptr() as usize;
        runtime[(vm_offset - (3 * POINTER_SIZE)) / POINTER_SIZE] =
            invalid_class_linker.as_mut_ptr() as usize;

        valid_class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        valid_class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        valid_class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        valid_class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        valid_class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());

        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: invalid_class_linker.as_ptr() as usize,
                    end: invalid_class_linker.as_ptr() as usize + invalid_class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: valid_class_linker.as_ptr() as usize,
                    end: valid_class_linker.as_ptr() as usize + valid_class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        let (layout, trampolines) = detect_runtime_layout_and_trampolines_from_runtime(
            30,
            runtime.as_mut_ptr().cast(),
            vm_value,
            None,
            None,
            &memory,
            FEATURE_METHOD_REPLACEMENT,
        )
        .unwrap();

        assert_eq!(layout.class_linker, valid_class_linker.as_mut_ptr().cast());
        assert_eq!(
            trampolines.quick_generic_jni_trampoline,
            quick_generic_jni as *mut c_void
        );
    }

    #[test]
    fn direct_jni_method_ids_are_not_decoded() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: Some(K_POINTER_JNI_ID_TYPE),
        };
        let method_id = 0x5555usize as jni::jmethodID;

        assert_eq!(
            backend.art_method_from_jni_id(&layout, method_id),
            vec![method_id.cast::<c_void>()]
        );
    }

    #[test]
    fn unknown_jni_method_id_mode_tries_raw_and_decoded_candidates() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: None,
        };
        let method_id = 0x5555usize as jni::jmethodID;

        assert_eq!(
            backend.art_method_from_jni_id(&layout, method_id),
            vec![method_id.cast::<c_void>(), 0x1234usize as *mut c_void]
        );
    }

    #[test]
    fn indirect_jni_method_ids_are_decoded() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: Some(1),
        };

        assert_eq!(
            backend.art_method_from_jni_id(&layout, 0x5555usize as jni::jmethodID),
            vec![0x1234usize as *mut c_void]
        );
    }

    #[test]
    fn detects_thread_class_method_array_layout_from_known_method() {
        let method_size = 24;
        let method_count = 3usize;
        let mut class_object = vec![0u8; CLASS_LAYOUT_SCAN_LIMIT];
        let mut methods = vec![0u8; POINTER_SIZE + (method_count * method_size)];
        let methods_offset = 32;
        let copied_methods_offset = 44;
        let methods_header = methods.as_mut_ptr() as usize;
        let known_method = unsafe {
            methods
                .as_mut_ptr()
                .byte_add(POINTER_SIZE + method_size)
                .cast::<c_void>()
        };
        class_object[methods_offset..methods_offset + POINTER_SIZE]
            .copy_from_slice(&methods_header.to_ne_bytes());
        methods[..POINTER_SIZE].copy_from_slice(&method_count.to_ne_bytes());
        class_object[copied_methods_offset..copied_methods_offset + 2]
            .copy_from_slice(&(method_count as u16).to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_object.as_ptr() as usize,
                    end: class_object.as_ptr() as usize + class_object.len(),
                    executable: false,
                },
                MemoryRange {
                    start: methods.as_ptr() as usize,
                    end: methods.as_ptr() as usize + methods.len(),
                    executable: false,
                },
            ],
        };

        let layout = detect_thread_class_method_layout(
            class_object.as_mut_ptr().cast(),
            &[vec![known_method]],
            method_size,
            &memory,
        )
        .unwrap();

        assert_eq!(layout.class_methods_offset, methods_offset);
        assert_eq!(layout.class_copied_methods_offset, copied_methods_offset);
        assert_eq!(layout.method_size, method_size);
    }

    #[test]
    fn detects_art_method_runtime_layout_from_access_flags_and_entrypoints() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let jni_code = code.as_mut_ptr() as usize;
        let quick_code = unsafe { code.as_mut_ptr().add(16) as usize };
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_NATIVE;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&jni_code.to_ne_bytes());
        method[quick_code_offset..quick_code_offset + POINTER_SIZE]
            .copy_from_slice(&quick_code.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn detects_art_method_replacement_layout_from_runtime_native_entrypoint() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE | 0x8000_0000;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&native_entrypoint.to_ne_bytes());
        method[quick_code_offset..quick_code_offset + POINTER_SIZE]
            .copy_from_slice(&(0x5555usize).to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn snapshots_patches_and_restores_art_method_fields() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: Some(32),
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC
                | K_ACC_STATIC
                | K_ACC_FINAL
                | K_ACC_FAST_NATIVE
                | K_ACC_CRITICAL_NATIVE
                | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
                | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
                | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
                | K_ACC_SINGLE_IMPLEMENTATION
                | K_ACC_SKIP_ACCESS_CHECKS,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );

        let patched = patched_replacement_method(
            original,
            0x4444usize as *mut c_void,
            0x5555usize as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &layout, patched);
        let patched_snapshot =
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory).unwrap();
        assert_eq!(patched_snapshot.jni_code, 0x4444usize as *mut c_void);
        assert_eq!(patched_snapshot.quick_code, 0x5555usize as *mut c_void);
        assert_eq!(
            patched_snapshot.interpreter_code,
            Some(0x3333usize as *mut c_void)
        );
        assert_ne!(patched_snapshot.access_flags & K_ACC_NATIVE, 0);
        assert_ne!(
            patched_snapshot.access_flags & compile_dont_bother_flag(30),
            0
        );
        assert_eq!(patched_snapshot.access_flags & K_ACC_FAST_NATIVE, 0);
        assert_eq!(patched_snapshot.access_flags & K_ACC_CRITICAL_NATIVE, 0);
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_NTERP_INVOKE_FAST_PATH_FLAG,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_SINGLE_IMPLEMENTATION,
            0
        );
        assert_eq!(patched_snapshot.access_flags & K_ACC_SKIP_ACCESS_CHECKS, 0);

        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn verified_patch_restores_original_on_mismatch() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let mismatched = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE,
            jni_code: 0x3333usize as *mut c_void,
            quick_code: 0x4444usize as *mut c_void,
            interpreter_code: Some(0x5555usize as *mut c_void),
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let error = patch_art_method_verified(
            method.as_mut_ptr().cast(),
            &layout,
            original,
            mismatched,
            &memory,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                ..
            }
        ));
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn verified_restore_checks_restored_snapshot() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let patched = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE,
            jni_code: 0x3333usize as *mut c_void,
            quick_code: 0x4444usize as *mut c_void,
            interpreter_code: None,
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, patched);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        restore_art_method_verified(method.as_mut_ptr().cast(), &layout, original, &memory)
            .unwrap();
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn cloned_art_method_copies_original_bytes() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let cloned = ArtMethodClone::copy_from(method.as_mut_ptr().cast(), &layout, &memory)
            .expect("ArtMethod clone allocation failed");
        let clone_memory = cloned.memory_ranges();
        assert_eq!(
            snapshot_art_method(cloned.as_ptr(), &layout, &clone_memory),
            Ok(original)
        );
        let original_bytes = &method[..layout.method_size];
        let cloned_bytes =
            unsafe { std::slice::from_raw_parts(cloned.as_ptr().cast::<u8>(), layout.method_size) };
        assert_eq!(cloned_bytes, original_bytes);
    }

    #[test]
    fn cloned_replacement_method_patches_clone_without_touching_original() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: Some(32),
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };
        let patched = patched_replacement_method(
            original,
            0x4444usize as *mut c_void,
            0x5555usize as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let cloned = clone_replacement_art_method(
            method.as_mut_ptr().cast(),
            &layout,
            original,
            patched,
            &memory,
        )
        .expect("replacement ArtMethod clone failed");
        let clone_memory = cloned.memory_ranges();
        assert_eq!(
            snapshot_art_method(cloned.as_ptr(), &layout, &clone_memory),
            Ok(patched)
        );
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
        drop(cloned);
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn original_clone_dispatch_patch_preserves_jni_entrypoint() {
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC
                | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
                | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
                | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
                | K_ACC_SINGLE_IMPLEMENTATION
                | K_ACC_SKIP_ACCESS_CHECKS,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };

        let patched = patched_original_method_for_clone_dispatch(
            original,
            QUICK_TO_INTERPRETER_TEST_STUB as *mut c_void,
            compile_dont_bother_flag(30),
        );

        assert_eq!(patched.jni_code, original.jni_code);
        assert_eq!(
            patched.quick_code,
            QUICK_TO_INTERPRETER_TEST_STUB as *mut c_void
        );
        assert_eq!(patched.interpreter_code, original.interpreter_code);
        assert_eq!(patched.access_flags & K_ACC_PUBLIC, K_ACC_PUBLIC);
        assert_eq!(
            patched.access_flags & K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE,
            0
        );
        assert_eq!(
            patched.access_flags & K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG,
            0
        );
        assert_eq!(patched.access_flags & K_ACC_SINGLE_IMPLEMENTATION, 0);
        assert_eq!(patched.access_flags & K_ACC_SKIP_ACCESS_CHECKS, 0);
        assert_ne!(patched.access_flags & compile_dont_bother_flag(30), 0);
    }

    #[test]
    fn detects_art_thread_managed_stack_offset_from_jni_env_field() {
        let mut thread = [0usize; 40];
        let env = 0x1234usize as *mut c_void;
        let jni_env_offset = 176;
        thread[jni_env_offset / POINTER_SIZE] = env as usize;

        let managed_stack_offset =
            detect_art_thread_managed_stack_offset("test feature", thread.as_mut_ptr().cast(), env)
                .expect("managed stack offset was not detected");

        assert_eq!(managed_stack_offset, jni_env_offset - (4 * POINTER_SIZE));
    }

    #[test]
    fn replacement_frame_detection_requires_linked_replacement_quick_frame() {
        let replacement = 0x1234_5678usize;
        let mut linked_quick_frame = [replacement];
        let mut linked_stack = [0usize; 3];
        linked_stack[0] = linked_quick_frame.as_mut_ptr() as usize | 1;

        let managed_stack_offset = 4 * POINTER_SIZE;
        let mut thread = [0usize; 16];
        thread[(managed_stack_offset + POINTER_SIZE) / POINTER_SIZE] =
            linked_stack.as_mut_ptr() as usize;

        assert!(replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));

        let mut current_quick_frame = [0xabcdusize];
        thread[managed_stack_offset / POINTER_SIZE] = current_quick_frame.as_mut_ptr() as usize;
        assert!(!replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));

        current_quick_frame[0] = replacement;
        assert_eq!(current_quick_frame[0], replacement);
        assert!(!replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));
    }

    #[test]
    fn replacement_controller_translates_registered_methods() {
        let controller = ArtReplacementController::empty_for_tests();
        let original = 0x1000usize as *mut c_void;
        let replacement = 0x2000usize as *mut c_void;

        assert_eq!(
            controller.translate_method_argument(original as usize),
            original as usize
        );
        controller
            .register(
                original,
                replacement,
                0x5000usize as *mut c_void,
                0x1000,
                ArtReplacementSynchronization {
                    quick_code_offset: POINTER_SIZE,
                    thread_managed_stack_offset: 0,
                    nterp_entrypoint: None,
                    quick_to_interpreter_bridge: 0,
                },
            )
            .expect("replacement registration should succeed");
        let jni_id = 0x3000usize as jni::jmethodID;
        controller.register_jni_id(jni_id, original);
        assert_eq!(
            controller.translate_method_argument(original as usize),
            replacement as usize
        );
        assert!(controller.has_dispatch_thunk_pc(original, 0x5000));
        assert!(controller.has_dispatch_thunk_pc(original, 0x5fff));
        assert!(!controller.has_dispatch_thunk_pc(original, 0x6000));
        assert!(!controller.has_dispatch_thunk_pc(replacement, 0x5000));
        assert_eq!(
            controller.art_method_for_jni_id(jni_id as usize),
            original as usize
        );
        assert_eq!(controller.art_method_for_jni_id(0x4444), 0x4444);
        assert!(controller.is_replacement_method(replacement));
        assert!(!controller.is_replacement_method(original));
        controller.unregister(original);
        assert_eq!(
            controller.translate_method_argument(original as usize),
            original as usize
        );
        assert_eq!(
            controller.art_method_for_jni_id(jni_id as usize),
            jni_id as usize
        );
        assert!(!controller.is_replacement_method(replacement));
        assert!(!controller.has_dispatch_thunk_pc(original, 0x5000));
    }

    #[test]
    fn replacement_controller_rejects_duplicate_active_replacement() {
        let controller = ArtReplacementController::empty_for_tests();
        let original = 0x1000usize as *mut c_void;
        let replacement = 0x2000usize as *mut c_void;
        let synchronization = ArtReplacementSynchronization {
            quick_code_offset: POINTER_SIZE,
            thread_managed_stack_offset: 0,
            nterp_entrypoint: None,
            quick_to_interpreter_bridge: 0,
        };

        controller
            .register(
                original,
                replacement,
                0x5000usize as *mut c_void,
                0x1000,
                synchronization,
            )
            .expect("first replacement registration should succeed");
        assert_eq!(
            controller.register(
                original,
                0x3000usize as *mut c_void,
                0x6000usize as *mut c_void,
                0x1000,
                synchronization,
            ),
            Err(Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "target ArtMethod already has an active replacement".to_owned(),
            })
        );
        assert_eq!(
            controller.register(
                0x4000usize as *mut c_void,
                replacement,
                0x6000usize as *mut c_void,
                0x1000,
                synchronization,
            ),
            Err(Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "replacement ArtMethod is already registered".to_owned(),
            })
        );
    }

    #[test]
    fn original_call_bypass_restores_method_and_thread() {
        let previous_method = ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst);
        let previous_thread = ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst);
        let previous_owner = ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst);

        {
            let _bypass = original_method_call_bypass(0x1111, 0x2222);
            assert_eq!(ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst), 0x1111);
            assert_eq!(ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst), 0x2222);
        }

        assert_eq!(
            ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst),
            previous_method
        );
        assert_eq!(
            ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst),
            previous_thread
        );
        assert_eq!(
            ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst),
            previous_owner
        );
    }

    #[test]
    fn original_call_bypass_allows_nested_calls_on_same_art_thread() {
        let previous_method = ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst);
        let previous_thread = ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst);
        let previous_owner = ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst);

        {
            let _outer = original_method_call_bypass(0x1111, 0x2222);
            assert_eq!(ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst), 0x1111);
            assert_eq!(ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst), 0x2222);

            {
                let _inner = original_method_call_bypass(0x3333, 0x2222);
                assert_eq!(ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst), 0x3333);
                assert_eq!(ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst), 0x2222);
            }

            assert_eq!(ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst), 0x1111);
            assert_eq!(ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst), 0x2222);
        }

        assert_eq!(
            ORIGINAL_CALL_BYPASS_METHOD.load(Ordering::SeqCst),
            previous_method
        );
        assert_eq!(
            ORIGINAL_CALL_BYPASS_THREAD.load(Ordering::SeqCst),
            previous_thread
        );
        assert_eq!(
            ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst),
            previous_owner
        );
    }

    #[test]
    fn replacement_controller_synchronizes_clone_declaring_class() {
        let controller = ArtReplacementController::empty_for_tests();
        let mut original = vec![0u8; 12];
        let mut replacement = vec![0u8; 12];
        let original_flags = K_ACC_PUBLIC | K_ACC_STATIC | compile_dont_bother_flag(30);
        write_u32(original.as_mut_ptr().cast(), 0xaaaa_bbbb);
        write_u32(
            unsafe { original.as_mut_ptr().byte_add(4).cast() },
            original_flags,
        );
        write_u32(replacement.as_mut_ptr().cast(), 0xcccc_dddd);
        write_u32(
            unsafe { replacement.as_mut_ptr().byte_add(4).cast() },
            K_ACC_NATIVE,
        );

        controller
            .register(
                original.as_mut_ptr().cast(),
                replacement.as_mut_ptr().cast(),
                0x5000usize as *mut c_void,
                0x1000,
                ArtReplacementSynchronization {
                    quick_code_offset: 8,
                    thread_managed_stack_offset: 0,
                    nterp_entrypoint: None,
                    quick_to_interpreter_bridge: 0,
                },
            )
            .expect("replacement registration should succeed");
        controller.synchronize_replacement_methods();

        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: replacement.as_ptr() as usize,
                end: replacement.as_ptr() as usize + replacement.len(),
                executable: false,
            }],
        };
        assert_eq!(
            read_u32(unsafe { replacement.as_ptr().byte_add(4).cast() }, &memory),
            Some(K_ACC_NATIVE)
        );
        assert_eq!(
            read_u32(replacement.as_ptr().cast(), &memory),
            Some(0xaaaa_bbbb)
        );
    }

    #[test]
    fn replacement_controller_rewrites_original_nterp_quick_code() {
        let controller = ArtReplacementController::empty_for_tests();
        let mut original = vec![0u8; 24];
        let mut replacement = vec![0u8; 24];
        let nterp = 0x1000usize;
        let quick_to_interpreter = 0x2000usize;
        write_usize(unsafe { original.as_mut_ptr().byte_add(16).cast() }, nterp);

        controller
            .register(
                original.as_mut_ptr().cast(),
                replacement.as_mut_ptr().cast(),
                0x5000usize as *mut c_void,
                0x1000,
                ArtReplacementSynchronization {
                    quick_code_offset: 16,
                    thread_managed_stack_offset: 0,
                    nterp_entrypoint: Some(nterp),
                    quick_to_interpreter_bridge: quick_to_interpreter,
                },
            )
            .expect("replacement registration should succeed");
        controller.synchronize_replacement_methods();

        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: original.as_ptr() as usize,
                end: original.as_ptr() as usize + original.len(),
                executable: false,
            }],
        };
        assert_eq!(
            read_usize(unsafe { original.as_ptr().byte_add(16).cast() }, &memory),
            Some(quick_to_interpreter)
        );
    }

    #[test]
    fn replacement_guard_debug_summary_includes_cloned_method() {
        let mut method = vec![0u8; 80];
        let method_layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let patched = patched_replacement_method(
            original,
            0x3333usize as *mut c_void,
            QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &method_layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };
        let cloned_method = clone_replacement_art_method(
            method.as_mut_ptr().cast(),
            &method_layout,
            original,
            patched,
            &memory,
        )
        .expect("replacement ArtMethod clone failed");
        let cloned_pointer = format!("{:?}", cloned_method.as_ptr());
        let dispatch_thunk = ArtMethodDispatchThunk::new(
            cloned_method.as_ptr(),
            original.quick_code,
            method_layout.quick_code_offset,
            160,
        )
        .expect("replacement dispatch thunk allocation failed");
        let original_patched = patched_original_method_for_clone_dispatch(
            original,
            dispatch_thunk.as_ptr(),
            compile_dont_bother_flag(30),
        );
        let guard = ArtMethodReplacementGuard {
            backend: ArtBackend::empty_for_tests(),
            vm: Vm::dangling_for_tests(),
            method: method.as_mut_ptr().cast(),
            cloned_method,
            dispatch_thunk,
            layout: ArtMethodReplacementLayout {
                api_level: 30,
                runtime: ArtRuntimeLayout {
                    runtime: 0x1000usize as *mut c_void,
                    heap: std::ptr::dangling_mut(),
                    thread_list: 0x2000usize as *mut c_void,
                    class_linker: 0x3000usize as *mut c_void,
                    intern_table: 0x4000usize as *mut c_void,
                    jni_id_manager: ptr::null_mut(),
                    jni_ids_indirection: None,
                },
                method: method_layout,
                trampolines: ArtClassLinkerTrampolines {
                    quick_resolution_trampoline: QUICK_RESOLUTION_TEST_STUB as *mut c_void,
                    quick_imt_conflict_trampoline: QUICK_IMT_CONFLICT_TEST_STUB as *mut c_void,
                    quick_generic_jni_trampoline: QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
                    quick_to_interpreter_bridge_trampoline: QUICK_TO_INTERPRETER_TEST_STUB
                        as *mut c_void,
                },
                thread_managed_stack_offset: 160,
            },
            original,
            original_patched,
            clone_patched: patched,
            reverted: true,
        };

        let summary = guard.debug_summary();
        assert!(summary.contains("backend=clone-active"));
        assert!(!summary.contains("clone-prepared-direct-active"));
        assert!(summary.contains("cloned_method="));
        assert!(summary.contains(&cloned_pointer));
        assert!(summary.contains("dispatch_thunk="));
        assert!(summary.contains("original_patched={access_flags="));
        assert!(summary.contains("clone_patched={access_flags="));
        assert!(summary.contains("quick_to_interpreter_bridge_trampoline="));
        assert!(summary.contains("thread_managed_stack_offset=160"));
        assert!(summary.contains("do_call_hooks=0"));
        assert!(summary.contains("quick_entrypoint_hooks=0"));
        assert!(summary.contains("get_oat_quick_method_header_hook=false"));
        assert!(summary.contains("gc_synchronization_hooks=0"));
    }

    #[test]
    fn rejects_non_executable_replacement_function() {
        let mut code = vec![0u8; 8];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: code.as_ptr() as usize,
                end: code.as_ptr() as usize + code.len(),
                executable: false,
            }],
        };

        assert_eq!(
            validate_replacement_function(code.as_mut_ptr().cast(), &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "replacement function is not executable".to_owned(),
            })
        );
    }

    #[test]
    fn accepts_executable_replacement_function() {
        let mut code = vec![0u8; 8];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: code.as_ptr() as usize,
                end: code.as_ptr() as usize + code.len(),
                executable: true,
            }],
        };

        assert_eq!(
            validate_replacement_function(code.as_mut_ptr().cast(), &memory),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_replacement_trampoline() {
        let trampolines = ArtClassLinkerTrampolines {
            quick_resolution_trampoline: 0x1000usize as *mut c_void,
            quick_imt_conflict_trampoline: 0x2000usize as *mut c_void,
            quick_generic_jni_trampoline: std::ptr::null_mut(),
            quick_to_interpreter_bridge_trampoline: 0x3000usize as *mut c_void,
        };
        let memory = MemoryRanges { ranges: Vec::new() };

        assert_eq!(
            validate_replacement_trampoline(&trampolines, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick generic JNI trampoline is unavailable or not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn rejects_null_replacement_function_before_runtime_work() {
        let backend = ArtBackend::empty_for_tests();
        let error = match backend.replace_method(
            &Vm::dangling_for_tests(),
            MethodKind::Static,
            0x1234usize as jni::jmethodID,
            std::ptr::null_mut(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("null replacement function unexpectedly succeeded"),
        };
        assert_eq!(
            error,
            Error::NullReturn {
                operation: "ART replacement function"
            }
        );
    }

    #[test]
    fn snapshot_rejects_unreadable_art_method() {
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let memory = MemoryRanges { ranges: Vec::new() };

        assert_eq!(
            snapshot_art_method(0x1234usize as *mut c_void, &layout, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "target ArtMethod is not readable".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_replacement_layout_without_runtime_native_entrypoint() {
        let mut method = vec![0u8; 80];
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&(0x7777usize).to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                ArtModuleRange {
                    start: 0x1000,
                    end: 0x2000,
                },
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ArtMethod runtime layout: native entrypoint is not executable"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn detects_replacement_layout_with_non_final_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE | 0x0008_0000;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&native_entrypoint.to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn rejects_replacement_layout_without_public_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&native_entrypoint.to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ArtMethod runtime layout: native access flags were not found"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn detects_replacement_layout_from_executable_entrypoint_fallback() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let native_entrypoint = unsafe { code.as_mut_ptr().add(16) as usize };
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&native_entrypoint.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                ArtModuleRange {
                    start: 0x1000,
                    end: 0x2000,
                },
                30,
                &memory,
                true,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn replacement_prerequisites_do_not_require_exception_clear_symbol() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.pretty_method = Some(PrettyMethodFunction {
            function: dummy_pretty_method,
            _thunk: None,
        });
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);
        backend.replacement_controller =
            std::sync::Arc::new(ArtReplacementController::with_dispatch_for_tests());

        assert_eq!(
            backend.method_replacement_support(&Vm::dangling_for_tests()),
            FeatureSupport::Unsupported {
                reason: "libandroid_runtime.so is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_art_method_layout_without_executable_entrypoints() {
        let mut method = vec![0u8; 80];
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_art_method_layout_without_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let jni_code = code.as_mut_ptr() as usize;
        let quick_code = unsafe { code.as_mut_ptr().add(16) as usize };
        method[24..24 + POINTER_SIZE].copy_from_slice(&jni_code.to_ne_bytes());
        method[24 + POINTER_SIZE..24 + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_code.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout".to_owned(),
            })
        );
    }

    #[test]
    fn detects_class_linker_trampolines_from_intern_table_anchor() {
        let mut class_linker = vec![0u8; 320];
        let mut code = vec![0u8; 96];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = code.as_mut_ptr() as usize;
        let quick_imt_conflict = unsafe { code.as_mut_ptr().add(16) as usize };
        let quick_generic_jni = unsafe { code.as_mut_ptr().add(32) as usize };
        let quick_to_interpreter = unsafe { code.as_mut_ptr().add(48) as usize };
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: quick_resolution as *mut c_void,
                quick_imt_conflict_trampoline: quick_imt_conflict as *mut c_void,
                quick_generic_jni_trampoline: quick_generic_jni as *mut c_void,
                quick_to_interpreter_bridge_trampoline: quick_to_interpreter as *mut c_void,
            })
        );
    }

    #[test]
    fn reports_missing_class_linker_intern_table_anchor() {
        let mut class_linker = vec![0u8; 1000];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: class_linker.as_ptr() as usize,
                end: class_linker.as_ptr() as usize + class_linker.len(),
                executable: false,
            }],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and ClassLinker quick-entrypoint predicate symbols are unavailable"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn detects_class_linker_trampolines_from_predicate_scan() {
        let mut class_linker = vec![0u8; 5000];
        let intern_table = 0x4444usize as *mut c_void;
        let quick_resolution_offset = 424;
        class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
            .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
        class_linker
            [quick_resolution_offset + POINTER_SIZE..quick_resolution_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
            ..quick_resolution_offset + (3 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
            ..quick_resolution_offset + (4 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: QUICK_RESOLUTION_TEST_STUB as *mut c_void,
                quick_imt_conflict_trampoline: QUICK_IMT_CONFLICT_TEST_STUB as *mut c_void,
                quick_generic_jni_trampoline: QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
                quick_to_interpreter_bridge_trampoline: QUICK_TO_INTERPRETER_TEST_STUB
                    as *mut c_void,
            })
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn detects_class_linker_trampolines_with_tagged_class_linker_pointer() {
        let mut class_linker = vec![0u8; 5000];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 424;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = QUICK_RESOLUTION_TEST_STUB;
        let quick_imt_conflict = QUICK_IMT_CONFLICT_TEST_STUB;
        let quick_generic_jni = QUICK_GENERIC_JNI_TEST_STUB;
        let quick_to_interpreter = QUICK_TO_INTERPRETER_TEST_STUB;
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let tagged_class_linker =
            ((class_linker.as_mut_ptr() as usize) | 0xab00_0000_0000_0000) as *mut c_void;
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: tagged_class_linker,
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: quick_resolution as *mut c_void,
                quick_imt_conflict_trampoline: quick_imt_conflict as *mut c_void,
                quick_generic_jni_trampoline: quick_generic_jni as *mut c_void,
                quick_to_interpreter_bridge_trampoline: quick_to_interpreter as *mut c_void,
            })
        );
    }

    #[test]
    fn reports_non_executable_predicate_trampoline() {
        let mut class_linker = vec![0u8; 5000];
        let quick_resolution_offset = 424;
        class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
            .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
        class_linker
            [quick_resolution_offset + POINTER_SIZE..quick_resolution_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
            ..quick_resolution_offset + (3 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
            ..quick_resolution_offset + (4 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: false,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick resolution trampoline at offset 0x1a8 is not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn reports_ambiguous_predicate_trampoline_candidates() {
        let mut class_linker = vec![0u8; 5000];
        for quick_resolution_offset in [424, 520] {
            class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
                .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + POINTER_SIZE
                ..quick_resolution_offset + (2 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
                ..quick_resolution_offset + (3 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
                ..quick_resolution_offset + (4 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        }
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ClassLinker trampoline offsets: predicate scan found multiple candidates"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn reports_non_executable_class_linker_trampoline() {
        let mut class_linker = vec![0u8; 320];
        let data = vec![0u8; 96];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = data.as_ptr() as usize;
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: data.as_ptr() as usize,
                    end: data.as_ptr() as usize + data.len(),
                    executable: false,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            heap: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick resolution trampoline at offset 0xe8 is not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn rejects_pre_api_26_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                25,
                std::ptr::dangling_mut::<usize>().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "Android API level 25 is below the API 26+ arm64 milestone".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_null_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                std::ptr::null_mut(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "ART Runtime pointer is null".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_unknown_runtime_layout() {
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "unable to determine ART Runtime field offsets".to_owned(),
            })
        );
    }

    #[test]
    fn maps_unsupported_support_to_matching_feature_error() {
        assert_eq!(
            ensure_feature_supported(
                FEATURE_CLASS_LOADER_ENUMERATION,
                FeatureSupport::Unsupported {
                    reason: "test reason".to_owned(),
                },
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "test reason".to_owned(),
            })
        );
    }

    #[test]
    fn initializes_class_loader_visitor_vtable_after_placement() {
        let mut loaders = Vec::new();
        let mut visitor = ArtClassLoaderVisitor::new(&mut loaders);
        assert!(visitor.vtable.is_null());

        visitor.initialize_vtable();

        assert_eq!(visitor.vtable, visitor.vtable_storage.as_ptr());
        assert_eq!(
            visitor.vtable_storage[2],
            on_visit_class_loader as *const c_void
        );
    }

    #[test]
    fn reports_missing_visit_class_loaders_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "VisitClassLoaders is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_suspend_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::SuspendAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_resume_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::ResumeAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_visit_classes_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ClassLinker::VisitClasses is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_loaded_class_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_heap_visitor_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.heap_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "Heap::VisitObjects and Heap::GetInstances are unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_heap_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_objects = Some(dummy_visit_objects);

        assert_eq!(
            backend.heap_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_heap_decode_global_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_objects = Some(dummy_visit_objects);
        backend.add_global_ref = Some(dummy_add_global_ref);

        assert_eq!(
            backend.heap_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::DecodeGlobal is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reads_mirror_object_class_reference() {
        let mut object = [0u32; 2];
        object[0] = 0x1234_5678;

        assert_eq!(
            object_class_reference(object.as_mut_ptr().cast()),
            0x1234_5678
        );
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn reports_non_arm64_architecture_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
    }
}
