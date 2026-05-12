use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    ptr::NonNull,
};

use crate::{
    env::Env,
    error::{Error, Result},
    jni,
};

#[cfg(target_arch = "aarch64")]
use frida_gum::instruction_writer::{
    Aarch64BranchCondition, Aarch64InstructionWriter, Aarch64Register, Argument, InstructionWriter,
};

const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const TRANSITION_CODE_SIZE: usize = 65536;
const JNIENV_EXT_SELF_OFFSET: u64 = POINTER_SIZE as u64;

type ThreadTransitionPerform = unsafe extern "C" fn(*mut jni::JNIEnv);
type RunnableCallback<'a> = dyn FnMut(*mut c_void) + 'a;

thread_local! {
    static RUNNABLE_CALLBACK: RefCell<Option<*mut RunnableCallback<'static>>> =
        RefCell::new(None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtThreadSpec {
    exception_offset: usize,
}

pub(super) struct ThreadTransition {
    perform: ThreadTransitionPerform,
    code: NonNull<c_void>,
}

unsafe impl Send for ThreadTransition {}
unsafe impl Sync for ThreadTransition {}

impl ThreadTransition {
    pub(super) fn run(
        &self,
        feature: &'static str,
        env: &Env<'_>,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let mut result = None;
        let mut f = Some(f);
        let mut callback = |thread| {
            if let Some(f) = f.take() {
                result = Some(f(thread));
            }
        };

        let callback: *mut RunnableCallback<'_> = &mut callback;
        let callback = unsafe {
            std::mem::transmute::<*mut RunnableCallback<'_>, *mut RunnableCallback<'static>>(
                callback,
            )
        };
        RUNNABLE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = Some(callback);
        });

        let unwind = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.perform)(env.handle().as_ptr());
        }));

        RUNNABLE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = None;
        });

        if unwind.is_err() {
            return unsupported(feature, "thread transition callback panicked");
        }

        result
            .unwrap_or_else(|| unsupported(feature, "unable to perform runnable thread transition"))
    }
}

impl Drop for ThreadTransition {
    fn drop(&mut self) {
        unsafe { frida_gum_sys::gum_free_pages(self.code.as_ptr()) };
    }
}

pub(super) fn build(
    feature: &'static str,
    env: &Env<'_>,
    exception_clear: Option<*const c_void>,
    fatal_error: Option<*const c_void>,
) -> Result<ThreadTransition> {
    if !cfg!(target_arch = "aarch64") {
        return unsupported(
            feature,
            "thread transition recompilation only supports arm64-v8a",
        );
    }

    let thread_spec = detect_thread_spec(feature, env)?;
    let exception_clear = exception_clear.unwrap_or_else(|| unsafe {
        jni::env_function::<*const c_void>(env.handle(), jni::ENV_EXCEPTION_CLEAR)
    });
    let fatal_error = fatal_error.unwrap_or_else(|| unsafe {
        jni::env_function::<*const c_void>(env.handle(), jni::ENV_FATAL_ERROR)
    });

    #[cfg(target_arch = "aarch64")]
    {
        build_arm64_thread_transition(feature, exception_clear, fatal_error, thread_spec)
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (exception_clear, fatal_error, thread_spec);
        unsupported(
            feature,
            "thread transition recompilation only supports arm64-v8a",
        )
    }
}

fn detect_thread_spec(feature: &'static str, env: &Env<'_>) -> Result<ArtThreadSpec> {
    let thread = art_thread_from_env(env);
    if thread.is_null() {
        return unsupported(feature, "ART Thread pointer is null");
    }

    detect_thread_exception_offset(feature, thread, env.handle().as_ptr().cast())
        .map(|exception_offset| ArtThreadSpec { exception_offset })
}

fn detect_thread_exception_offset(
    feature: &'static str,
    thread: *mut c_void,
    env: *mut c_void,
) -> Result<usize> {
    let thread = thread.cast::<usize>();
    let env_value = env as usize;
    for offset in (144..256).step_by(POINTER_SIZE) {
        let value = unsafe { thread.byte_add(offset).read() };
        if value == env_value {
            return Ok(offset - (6 * POINTER_SIZE));
        }
    }

    unsupported(feature, "unable to determine ArtThread field offsets")
}

fn art_thread_from_env(env: &Env<'_>) -> *mut c_void {
    unsafe { env.handle().as_ptr().cast::<*mut c_void>().add(1).read() }
}

unsafe extern "C" fn on_thread_transition_complete(thread: *mut c_void) {
    RUNNABLE_CALLBACK.with(|slot| {
        let Some(callback) = *slot.borrow() else {
            return;
        };

        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let callback = unsafe { &mut *callback };
            callback(thread);
        }));
    });
}

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arm64Insn {
    B { target: u64 },
    BCond { cond: u8, target: u64 },
    Cbz { reg: u8, is64: bool, target: u64 },
    Cbnz { reg: u8, is64: bool, target: u64 },
    Tbz { reg: u8, bit: u8, target: u64 },
    Tbnz { reg: u8, bit: u8, target: u64 },
    Ret,
    Str { rt: u8, rn: u8, offset: usize },
    Ldr { rt: u8, rn: u8, offset: usize },
    Blr { rn: u8 },
    Other,
}

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy)]
struct Arm64Block {
    begin: u64,
    end: u64,
}

#[cfg(target_arch = "aarch64")]
fn build_arm64_thread_transition(
    feature: &'static str,
    exception_clear: *const c_void,
    next_function: *const c_void,
    thread_spec: ArtThreadSpec,
) -> Result<ThreadTransition> {
    let page_size = unsafe { frida_gum_sys::gum_query_page_size() as usize };
    let pages = TRANSITION_CODE_SIZE.div_ceil(page_size);
    let code = unsafe {
        frida_gum_sys::gum_alloc_n_pages(
            pages as u32,
            (frida_gum_sys::_GumPageProtection_GUM_PAGE_READ
                | frida_gum_sys::_GumPageProtection_GUM_PAGE_WRITE
                | frida_gum_sys::_GumPageProtection_GUM_PAGE_EXECUTE)
                as frida_gum_sys::GumPageProtection,
        )
    };
    let Some(code) = NonNull::new(code) else {
        return unsupported(feature, "unable to allocate executable transition code");
    };

    match write_arm64_thread_transition(
        feature,
        code.as_ptr(),
        exception_clear,
        next_function,
        thread_spec,
    ) {
        Ok(perform) => Ok(ThreadTransition { perform, code }),
        Err(error) => {
            unsafe { frida_gum_sys::gum_free_pages(code.as_ptr()) };
            Err(error)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn write_arm64_thread_transition(
    feature: &'static str,
    code: *mut c_void,
    exception_clear: *const c_void,
    next_function: *const c_void,
    thread_spec: ArtThreadSpec,
) -> Result<ThreadTransitionPerform> {
    let (blocks, branch_targets) =
        collect_arm64_blocks(feature, exception_clear as u64, next_function as u64)?;
    let mut blocks_ordered = blocks.values().copied().collect::<Vec<_>>();
    blocks_ordered.sort_by_key(|block| block.begin);
    if let Some(entry_index) = blocks_ordered
        .iter()
        .position(|block| block.begin == exception_clear as u64)
    {
        let entry = blocks_ordered.remove(entry_index);
        blocks_ordered.insert(0, entry);
    }

    let writer = Aarch64InstructionWriter::new(code as u64);

    const PERFORM_TRANSITION: u64 = u64::MAX - 1;
    writer.put_b_label(PERFORM_TRANSITION);

    let invoke_callback = writer.pc();
    put_arm64_push_all_x(&writer);
    writer.put_call_address_with_arguments(
        on_thread_transition_complete as *const c_void as u64,
        &[Argument::Register(Aarch64Register::X0)],
    );
    put_arm64_pop_all_x(&writer);
    put_arm64_ret(&writer);

    writer.put_label(PERFORM_TRANSITION);

    let mut found_core = false;
    let mut thread_reg = None;
    let mut real_impl_reg = None;

    for block in blocks_ordered {
        let mut address = block.begin;

        while address < block.end {
            let word = unsafe { (address as *const u32).read() };
            let decoded = decode_arm64(address, word);
            if branch_targets.contains(&address) {
                writer.put_label(address);
            }

            let mut keep = true;
            match decoded {
                Arm64Insn::B { target } => {
                    writer.put_b_label(target);
                    keep = false;
                }
                Arm64Insn::BCond { cond, target } => {
                    writer.put_bcond_label(arm64_condition(cond), target);
                    keep = false;
                }
                Arm64Insn::Cbz { reg, is64, target } => {
                    put_arm64_cbz_label(&writer, arm64_register_id(reg, is64), target);
                    keep = false;
                }
                Arm64Insn::Cbnz { reg, is64, target } => {
                    put_arm64_cbnz_label(&writer, arm64_register_id(reg, is64), target);
                    keep = false;
                }
                Arm64Insn::Tbz { reg, bit, target } => {
                    put_arm64_tbz_label(&writer, arm64_register_id(reg, true), bit, target);
                    keep = false;
                }
                Arm64Insn::Tbnz { reg, bit, target } => {
                    put_arm64_tbnz_label(&writer, arm64_register_id(reg, true), bit, target);
                    keep = false;
                }
                Arm64Insn::Str { rt, rn, offset }
                    if rt == 31 && offset == thread_spec.exception_offset =>
                {
                    let rn = arm64_x_register(rn);
                    writer.put_push_reg_reg(Aarch64Register::X0, Aarch64Register::Lr);
                    writer.put_mov_reg_reg(Aarch64Register::X0, rn);
                    writer.put_bl_imm(invoke_callback);
                    writer.put_pop_reg_reg(Aarch64Register::X0, Aarch64Register::Lr);

                    thread_reg = Some(rn);
                    found_core = true;
                    keep = false;
                }
                Arm64Insn::Str { rn, offset, .. }
                    if thread_reg == Some(arm64_x_register(rn))
                        && is_neutered_thread_store(offset) =>
                {
                    keep = false;
                }
                Arm64Insn::Ldr { rt, offset, .. }
                    if offset == jni::ENV_EXCEPTION_CLEAR * POINTER_SIZE =>
                {
                    real_impl_reg = Some(rt);
                }
                Arm64Insn::Blr { rn } if real_impl_reg == Some(rn) => {
                    writer.put_ldr_reg_reg_offset(
                        Aarch64Register::X0,
                        Aarch64Register::X0,
                        JNIENV_EXT_SELF_OFFSET,
                    );
                    writer.put_call_address_with_arguments(
                        on_thread_transition_complete as *const c_void as u64,
                        &[Argument::Register(Aarch64Register::X0)],
                    );
                    real_impl_reg = None;
                    found_core = true;
                    keep = false;
                }
                _ => {}
            }

            if keep {
                writer.put_bytes(&word.to_le_bytes());
            }

            address += 4;
        }
    }

    writer.flush();
    unsafe { frida_gum_sys::gum_clear_cache(code, TRANSITION_CODE_SIZE as u64) };

    if !found_core {
        return unsupported(
            feature,
            "unable to parse ART ExceptionClear thread transition",
        );
    }

    Ok(unsafe { std::mem::transmute::<*mut c_void, ThreadTransitionPerform>(code) })
}

#[cfg(target_arch = "aarch64")]
fn collect_arm64_blocks(
    feature: &'static str,
    entry: u64,
    next_function: u64,
) -> Result<(HashMap<u64, Arm64Block>, HashSet<u64>)> {
    let mut blocks = HashMap::new();
    let mut branch_targets = HashSet::new();
    let mut pending = VecDeque::from([entry]);

    while let Some(mut current) = pending.pop_front() {
        if blocks
            .values()
            .any(|block: &Arm64Block| current >= block.begin && current < block.end)
        {
            continue;
        }

        let begin = current;
        let mut end = current;
        loop {
            if current == next_function {
                break;
            }

            let word = unsafe { (current as *const u32).read() };
            if word == 0 {
                break;
            }

            let decoded = decode_arm64(current, word);
            end = current + 4;
            if let Some(target) = decoded.branch_target() {
                branch_targets.insert(target);
                pending.push_back(target);
                pending.make_contiguous().sort_unstable();
            }

            current += 4;
            if decoded.ends_block() {
                break;
            }
        }

        if end == begin {
            return unsupported(feature, "unable to parse empty ART ExceptionClear block");
        }

        blocks.insert(begin, Arm64Block { begin, end });
    }

    if !blocks.contains_key(&entry) {
        return unsupported(feature, "unable to parse ART ExceptionClear entry block");
    }

    Ok((blocks, branch_targets))
}

#[cfg(target_arch = "aarch64")]
impl Arm64Insn {
    fn branch_target(self) -> Option<u64> {
        match self {
            Self::B { target }
            | Self::BCond { target, .. }
            | Self::Cbz { target, .. }
            | Self::Cbnz { target, .. }
            | Self::Tbz { target, .. }
            | Self::Tbnz { target, .. } => Some(target),
            _ => None,
        }
    }

    fn ends_block(self) -> bool {
        matches!(self, Self::B { .. } | Self::Ret)
    }
}

#[cfg(target_arch = "aarch64")]
fn decode_arm64(address: u64, word: u32) -> Arm64Insn {
    if word & 0x7c00_0000 == 0x1400_0000 {
        return Arm64Insn::B {
            target: arm64_branch_target(address, word & 0x03ff_ffff, 26),
        };
    }
    if word & 0xff00_0010 == 0x5400_0000 {
        return Arm64Insn::BCond {
            cond: (word & 0xf) as u8,
            target: arm64_branch_target(address, (word >> 5) & 0x7ffff, 19),
        };
    }
    if word & 0x7f00_0000 == 0x3400_0000 {
        return Arm64Insn::Cbz {
            reg: (word & 0x1f) as u8,
            is64: (word >> 31) != 0,
            target: arm64_branch_target(address, (word >> 5) & 0x7ffff, 19),
        };
    }
    if word & 0x7f00_0000 == 0x3500_0000 {
        return Arm64Insn::Cbnz {
            reg: (word & 0x1f) as u8,
            is64: (word >> 31) != 0,
            target: arm64_branch_target(address, (word >> 5) & 0x7ffff, 19),
        };
    }
    if word & 0x7f00_0000 == 0x3600_0000 {
        return Arm64Insn::Tbz {
            reg: (word & 0x1f) as u8,
            bit: (((word >> 26) & 0x20) | ((word >> 19) & 0x1f)) as u8,
            target: arm64_branch_target(address, (word >> 5) & 0x3fff, 14),
        };
    }
    if word & 0x7f00_0000 == 0x3700_0000 {
        return Arm64Insn::Tbnz {
            reg: (word & 0x1f) as u8,
            bit: (((word >> 26) & 0x20) | ((word >> 19) & 0x1f)) as u8,
            target: arm64_branch_target(address, (word >> 5) & 0x3fff, 14),
        };
    }
    if word & 0xffff_fc1f == 0xd65f_0000 {
        return Arm64Insn::Ret;
    }
    if word & 0xffc0_0000 == 0xf900_0000 {
        return Arm64Insn::Str {
            rt: (word & 0x1f) as u8,
            rn: ((word >> 5) & 0x1f) as u8,
            offset: (((word >> 10) & 0xfff) as usize) * 8,
        };
    }
    if word & 0xffe0_0c00 == 0xf800_0000 {
        return Arm64Insn::Str {
            rt: (word & 0x1f) as u8,
            rn: ((word >> 5) & 0x1f) as u8,
            offset: sign_extend((word >> 12) & 0x1ff, 9) as usize,
        };
    }
    if word & 0xffc0_0000 == 0xf940_0000 {
        return Arm64Insn::Ldr {
            rt: (word & 0x1f) as u8,
            rn: ((word >> 5) & 0x1f) as u8,
            offset: (((word >> 10) & 0xfff) as usize) * 8,
        };
    }
    if word & 0xffe0_0c00 == 0xf840_0000 {
        return Arm64Insn::Ldr {
            rt: (word & 0x1f) as u8,
            rn: ((word >> 5) & 0x1f) as u8,
            offset: sign_extend((word >> 12) & 0x1ff, 9) as usize,
        };
    }
    if word & 0xffff_fc1f == 0xd63f_0000 {
        return Arm64Insn::Blr {
            rn: ((word >> 5) & 0x1f) as u8,
        };
    }

    Arm64Insn::Other
}

#[cfg(target_arch = "aarch64")]
fn arm64_branch_target(address: u64, immediate: u32, bits: u8) -> u64 {
    address.wrapping_add((sign_extend(immediate, bits) << 2) as u64)
}

#[cfg(target_arch = "aarch64")]
fn sign_extend(value: u32, bits: u8) -> i64 {
    let shift = 64 - bits;
    ((value as i64) << shift) >> shift
}

#[cfg(target_arch = "aarch64")]
fn is_neutered_thread_store(_offset: usize) -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
fn arm64_condition(condition: u8) -> Aarch64BranchCondition {
    match condition {
        0 => Aarch64BranchCondition::Eq,
        1 => Aarch64BranchCondition::Ne,
        2 => Aarch64BranchCondition::Hs,
        3 => Aarch64BranchCondition::Lo,
        4 => Aarch64BranchCondition::Mi,
        5 => Aarch64BranchCondition::Pl,
        6 => Aarch64BranchCondition::Vs,
        7 => Aarch64BranchCondition::Vc,
        8 => Aarch64BranchCondition::Hi,
        9 => Aarch64BranchCondition::Ls,
        10 => Aarch64BranchCondition::Ge,
        11 => Aarch64BranchCondition::Lt,
        12 => Aarch64BranchCondition::Gt,
        13 => Aarch64BranchCondition::Le,
        14 => Aarch64BranchCondition::Al,
        _ => Aarch64BranchCondition::Nv,
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_register_id(encoded: u8, is64: bool) -> u32 {
    if is64 {
        arm64_x_register(encoded) as u32
    } else {
        arm64_w_register(encoded) as u32
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_x_register(encoded: u8) -> Aarch64Register {
    match encoded {
        0 => Aarch64Register::X0,
        1 => Aarch64Register::X1,
        2 => Aarch64Register::X2,
        3 => Aarch64Register::X3,
        4 => Aarch64Register::X4,
        5 => Aarch64Register::X5,
        6 => Aarch64Register::X6,
        7 => Aarch64Register::X7,
        8 => Aarch64Register::X8,
        9 => Aarch64Register::X9,
        10 => Aarch64Register::X10,
        11 => Aarch64Register::X11,
        12 => Aarch64Register::X12,
        13 => Aarch64Register::X13,
        14 => Aarch64Register::X14,
        15 => Aarch64Register::X15,
        16 => Aarch64Register::X16,
        17 => Aarch64Register::X17,
        18 => Aarch64Register::X18,
        19 => Aarch64Register::X19,
        20 => Aarch64Register::X20,
        21 => Aarch64Register::X21,
        22 => Aarch64Register::X22,
        23 => Aarch64Register::X23,
        24 => Aarch64Register::X24,
        25 => Aarch64Register::X25,
        26 => Aarch64Register::X26,
        27 => Aarch64Register::X27,
        28 => Aarch64Register::X28,
        29 => Aarch64Register::Fp,
        30 => Aarch64Register::Lr,
        _ => Aarch64Register::Xzr,
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_w_register(encoded: u8) -> Aarch64Register {
    match encoded {
        0 => Aarch64Register::W0,
        1 => Aarch64Register::W1,
        2 => Aarch64Register::W2,
        3 => Aarch64Register::W3,
        4 => Aarch64Register::W4,
        5 => Aarch64Register::W5,
        6 => Aarch64Register::W6,
        7 => Aarch64Register::W7,
        8 => Aarch64Register::W8,
        9 => Aarch64Register::W9,
        10 => Aarch64Register::W10,
        11 => Aarch64Register::W11,
        12 => Aarch64Register::W12,
        13 => Aarch64Register::W13,
        14 => Aarch64Register::W14,
        15 => Aarch64Register::W15,
        16 => Aarch64Register::W16,
        17 => Aarch64Register::W17,
        18 => Aarch64Register::W18,
        19 => Aarch64Register::W19,
        20 => Aarch64Register::W20,
        21 => Aarch64Register::W21,
        22 => Aarch64Register::W22,
        23 => Aarch64Register::W23,
        24 => Aarch64Register::W24,
        25 => Aarch64Register::W25,
        26 => Aarch64Register::W26,
        27 => Aarch64Register::W27,
        28 => Aarch64Register::W28,
        29 => Aarch64Register::W29,
        30 => Aarch64Register::W30,
        _ => Aarch64Register::Wzr,
    }
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_ret(writer: &Aarch64InstructionWriter) {
    writer.put_bytes(&0xd65f_03c0_u32.to_le_bytes());
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_push_all_x(writer: &Aarch64InstructionWriter) {
    unsafe { frida_gum_sys::gum_arm64_writer_put_push_all_x_registers(writer.raw_writer()) };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_pop_all_x(writer: &Aarch64InstructionWriter) {
    unsafe { frida_gum_sys::gum_arm64_writer_put_pop_all_x_registers(writer.raw_writer()) };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_cbz_label(writer: &Aarch64InstructionWriter, reg: u32, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_cbz_reg_label(
            writer.raw_writer(),
            reg,
            label as *const c_void,
        )
    };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_cbnz_label(writer: &Aarch64InstructionWriter, reg: u32, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_cbnz_reg_label(
            writer.raw_writer(),
            reg,
            label as *const c_void,
        )
    };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_tbz_label(writer: &Aarch64InstructionWriter, reg: u32, bit: u8, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_tbz_reg_imm_label(
            writer.raw_writer(),
            reg,
            bit as u32,
            label as *const c_void,
        )
    };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_tbnz_label(writer: &Aarch64InstructionWriter, reg: u32, bit: u8, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_tbnz_reg_imm_label(
            writer.raw_writer(),
            reg,
            bit as u32,
            label as *const c_void,
        )
    };
}

fn unsupported<T>(feature: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_thread_exception_offset_from_jni_env_field() {
        let mut thread = [0usize; 40];
        let env = 0x1234usize as *mut c_void;
        let jni_env_offset = 160;
        thread[jni_env_offset / POINTER_SIZE] = env as usize;

        let exception_offset =
            detect_thread_exception_offset("test feature", thread.as_mut_ptr().cast(), env)
                .unwrap();

        assert_eq!(exception_offset, jni_env_offset - (6 * POINTER_SIZE));
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn decodes_arm64_thread_transition_instructions() {
        assert_eq!(
            decode_arm64(0x1000, 0x1400_0002),
            Arm64Insn::B { target: 0x1008 }
        );
        assert_eq!(
            decode_arm64(0x1000, 0x5400_0041),
            Arm64Insn::BCond {
                cond: 1,
                target: 0x1008
            }
        );
        assert_eq!(
            decode_arm64(0x1000, 0xb400_0040),
            Arm64Insn::Cbz {
                reg: 0,
                is64: true,
                target: 0x1008
            }
        );
        assert_eq!(
            decode_arm64(0x1000, 0xf900_083f),
            Arm64Insn::Str {
                rt: 31,
                rn: 1,
                offset: 16
            }
        );
        assert_eq!(
            decode_arm64(0x1000, 0xf940_4402),
            Arm64Insn::Ldr {
                rt: 2,
                rn: 0,
                offset: 136
            }
        );
        assert_eq!(decode_arm64(0x1000, 0xd63f_0060), Arm64Insn::Blr { rn: 3 });
    }
}
