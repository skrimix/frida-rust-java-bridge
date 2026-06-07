use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::c_void,
    ptr::NonNull,
};

use crate::{
    error::{Error, Result},
    jni,
};
use frida_gum::instruction_writer::{
    Aarch64InstructionWriter, Aarch64Register, Argument, InstructionWriter,
};
use frida_gum_sys as gum_sys;

use super::{
    ArtThreadSpec, JNIENV_EXT_SELF_OFFSET, POINTER_SIZE, RunnableThreadTransition,
    RunnableThreadTransitionPerform, TRANSITION_CODE_SIZE, on_runnable_thread_transition_complete,
    unsupported,
};

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arm64Insn {
    B { target: u64 },
    BCond { cond: u32, target: u64 },
    Cbz { reg: u32, target: u64 },
    Cbnz { reg: u32, target: u64 },
    Tbz { reg: u32, bit: u32, target: u64 },
    Tbnz { reg: u32, bit: u32, target: u64 },
    Ret,
    Str { rt: u32, rn: u32, offset: i64 },
    Ldr { rt: u32, rn: u32, offset: i64 },
    Blr { rn: u32 },
    Other,
}

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy)]
struct Arm64Block {
    begin: u64,
    end: u64,
}

#[cfg(target_arch = "aarch64")]
struct RawArm64Relocator {
    inner: *mut c_void,
}

#[cfg(target_arch = "aarch64")]
impl RawArm64Relocator {
    fn new(input_code: u64, output: Option<&Aarch64InstructionWriter>) -> Self {
        unsafe extern "C" {
            fn gum_arm64_relocator_new(
                input_code: *const c_void,
                output: *mut gum_sys::_GumArm64Writer,
            ) -> *mut c_void;
        }

        let output = output.map_or(std::ptr::null_mut(), Aarch64InstructionWriter::raw_writer);
        Self {
            inner: unsafe { gum_arm64_relocator_new(input_code as *const c_void, output) },
        }
    }

    fn read_one(&mut self) -> (u32, *const gum_sys::cs_insn) {
        unsafe extern "C" {
            fn gum_arm64_relocator_read_one(
                relocator: *mut c_void,
                instruction: *mut *const gum_sys::cs_insn,
            ) -> u32;
        }

        let mut instruction = std::ptr::null();
        let offset = unsafe { gum_arm64_relocator_read_one(self.inner, &mut instruction) };
        (offset, instruction)
    }

    fn skip_one(&mut self) {
        unsafe extern "C" {
            fn gum_arm64_relocator_skip_one(relocator: *mut c_void);
        }

        unsafe { gum_arm64_relocator_skip_one(self.inner) };
    }

    fn write_all(&mut self) {
        unsafe extern "C" {
            fn gum_arm64_relocator_write_all(relocator: *mut c_void);
        }

        unsafe { gum_arm64_relocator_write_all(self.inner) };
    }
}

#[cfg(target_arch = "aarch64")]
impl Drop for RawArm64Relocator {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn gum_arm64_relocator_unref(relocator: *mut c_void);
        }

        unsafe { gum_arm64_relocator_unref(self.inner) };
    }
}

#[cfg(target_arch = "aarch64")]
pub(super) fn build_thread_transition(
    feature: &'static str,
    exception_clear: *const c_void,
    next_function: *const c_void,
    thread_spec: ArtThreadSpec,
) -> Result<RunnableThreadTransition> {
    let _gum = crate::native::process_gum();
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

    match write_thread_transition(
        feature,
        code.as_ptr(),
        exception_clear,
        next_function,
        thread_spec,
    ) {
        Ok(perform) => Ok(RunnableThreadTransition { perform, code }),
        Err(error) => {
            unsafe { frida_gum_sys::gum_free_pages(code.as_ptr()) };
            Err(error)
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn write_thread_transition(
    feature: &'static str,
    code: *mut c_void,
    exception_clear: *const c_void,
    next_function: *const c_void,
    thread_spec: ArtThreadSpec,
) -> Result<RunnableThreadTransitionPerform> {
    let _gum = crate::native::process_gum();
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
        on_runnable_thread_transition_complete as *const c_void as u64,
        &[Argument::Register(Aarch64Register::X0)],
    );
    put_arm64_pop_all_x(&writer);
    put_arm64_ret(&writer);

    writer.put_label(PERFORM_TRANSITION);

    let mut found_core = false;
    let mut thread_reg = None;
    let mut real_impl_reg = None;

    for block in blocks_ordered {
        let size = (block.end - block.begin) as u32;
        let mut relocator = RawArm64Relocator::new(block.begin, Some(&writer));

        loop {
            let (offset, instruction) = relocator.read_one();
            if offset == 0 || offset > size || instruction.is_null() {
                let address = block.begin + offset.saturating_sub(4) as u64;
                let word = if offset >= 4 && offset <= size {
                    unsafe { (address as *const u32).read() }
                } else {
                    0
                };
                return unsupported(
                    feature,
                    format!(
                        "unable to relocate ART ExceptionClear instruction: block {:#x}..{:#x}, offset {}, word {:#010x}",
                        block.begin, block.end, offset, word
                    ),
                );
            }

            let address = block.begin + offset as u64 - 4;
            let decoded = decode_arm64(feature, instruction)?;
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
                    put_arm64_bcond_label(&writer, cond, target);
                    keep = false;
                }
                Arm64Insn::Cbz { reg, target } => {
                    put_arm64_cbz_label(&writer, reg, target);
                    keep = false;
                }
                Arm64Insn::Cbnz { reg, target } => {
                    put_arm64_cbnz_label(&writer, reg, target);
                    keep = false;
                }
                Arm64Insn::Tbz { reg, bit, target } => {
                    put_arm64_tbz_label(&writer, reg, bit, target);
                    keep = false;
                }
                Arm64Insn::Tbnz { reg, bit, target } => {
                    put_arm64_tbnz_label(&writer, reg, bit, target);
                    keep = false;
                }
                Arm64Insn::Str { rt, rn, offset }
                    if is_arm64_zero_register(rt)
                        && offset == thread_spec.exception_offset as i64 =>
                {
                    writer.put_push_reg_reg(Aarch64Register::X0, Aarch64Register::Lr);
                    put_arm64_mov_reg_reg(&writer, Aarch64Register::X0 as u32, rn);
                    writer.put_bl_imm(invoke_callback);
                    writer.put_pop_reg_reg(Aarch64Register::X0, Aarch64Register::Lr);

                    thread_reg = Some(rn);
                    found_core = true;
                    keep = false;
                }
                Arm64Insn::Str { rn, offset, .. }
                    if thread_reg == Some(rn) && is_neutered_thread_store(offset) =>
                {
                    keep = false;
                }
                Arm64Insn::Ldr { rt, offset, .. }
                    if offset == (jni::ENV_EXCEPTION_CLEAR * POINTER_SIZE) as i64 =>
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
                        on_runnable_thread_transition_complete as *const c_void as u64,
                        &[Argument::Register(Aarch64Register::X0)],
                    );
                    real_impl_reg = None;
                    found_core = true;
                    keep = false;
                }
                _ => {}
            }

            if keep {
                relocator.write_all();
            } else {
                relocator.skip_one();
            }

            if offset == size {
                break;
            }
        }
    }

    writer.flush();
    unsafe { frida_gum_sys::gum_clear_cache(code, TRANSITION_CODE_SIZE as u64) };

    if !found_core {
        return unsupported(
            feature,
            "unable to parse ART ExceptionClear runnable thread transition",
        );
    }

    Ok(unsafe { std::mem::transmute::<*mut c_void, RunnableThreadTransitionPerform>(code) })
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

    while let Some(current) = pending.pop_front() {
        if blocks
            .values()
            .any(|block: &Arm64Block| current >= block.begin && current < block.end)
        {
            continue;
        }

        let begin = current;
        let mut end = current;
        let mut relocator = RawArm64Relocator::new(begin, None);
        loop {
            let (offset, instruction) = relocator.read_one();
            if offset == 0 || instruction.is_null() {
                break;
            }

            let address = begin + offset as u64 - unsafe { (*instruction).size as u64 };
            if address == next_function {
                break;
            }

            let decoded = decode_arm64(feature, instruction)?;
            end = begin + offset as u64;
            if let Some(target) = decoded.branch_target() {
                branch_targets.insert(target);
                pending.push_back(target);
                pending.make_contiguous().sort_unstable();
            }

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
fn decode_arm64(feature: &'static str, instruction: *const gum_sys::cs_insn) -> Result<Arm64Insn> {
    let instruction = unsafe { &*instruction };
    let detail = NonNull::new(instruction.detail).ok_or_else(|| Error::UnsupportedFeature {
        feature,
        reason: format!(
            "unable to decode ART ExceptionClear instruction detail at {:#x}",
            instruction.address
        ),
    })?;
    let arm64 = unsafe { detail.as_ref().__bindgen_anon_1.arm64 };
    let operands = &arm64.operands[..arm64.op_count as usize];

    Ok(match instruction.id {
        gum_sys::arm64_insn_ARM64_INS_B => {
            let target = operand_imm(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear branch target at {:#x}",
                    instruction.address
                ),
            })? as u64;

            if arm64.cc == gum_sys::arm64_cc_ARM64_CC_INVALID {
                Arm64Insn::B { target }
            } else {
                Arm64Insn::BCond {
                    cond: arm64.cc,
                    target,
                }
            }
        }
        gum_sys::arm64_insn_ARM64_INS_CBZ => Arm64Insn::Cbz {
            reg: operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear cbz register at {:#x}",
                    instruction.address
                ),
            })?,
            target: operand_imm(operands, 1).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear cbz target at {:#x}",
                    instruction.address
                ),
            })? as u64,
        },
        gum_sys::arm64_insn_ARM64_INS_CBNZ => Arm64Insn::Cbnz {
            reg: operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear cbnz register at {:#x}",
                    instruction.address
                ),
            })?,
            target: operand_imm(operands, 1).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear cbnz target at {:#x}",
                    instruction.address
                ),
            })? as u64,
        },
        gum_sys::arm64_insn_ARM64_INS_TBZ => Arm64Insn::Tbz {
            reg: operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbz register at {:#x}",
                    instruction.address
                ),
            })?,
            bit: operand_imm(operands, 1).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbz bit at {:#x}",
                    instruction.address
                ),
            })? as u32,
            target: operand_imm(operands, 2).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbz target at {:#x}",
                    instruction.address
                ),
            })? as u64,
        },
        gum_sys::arm64_insn_ARM64_INS_TBNZ => Arm64Insn::Tbnz {
            reg: operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbnz register at {:#x}",
                    instruction.address
                ),
            })?,
            bit: operand_imm(operands, 1).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbnz bit at {:#x}",
                    instruction.address
                ),
            })? as u32,
            target: operand_imm(operands, 2).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear tbnz target at {:#x}",
                    instruction.address
                ),
            })? as u64,
        },
        gum_sys::arm64_insn_ARM64_INS_RET
        | gum_sys::arm64_insn_ARM64_INS_RETAA
        | gum_sys::arm64_insn_ARM64_INS_RETAB => Arm64Insn::Ret,
        gum_sys::arm64_insn_ARM64_INS_STR => decode_arm64_memory(feature, instruction, operands)
            .map(|(rt, rn, offset)| Arm64Insn::Str { rt, rn, offset })?,
        gum_sys::arm64_insn_ARM64_INS_LDR => decode_arm64_memory(feature, instruction, operands)
            .map(|(rt, rn, offset)| Arm64Insn::Ldr { rt, rn, offset })?,
        gum_sys::arm64_insn_ARM64_INS_BLR => Arm64Insn::Blr {
            rn: operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
                feature,
                reason: format!(
                    "unable to decode ART ExceptionClear blr register at {:#x}",
                    instruction.address
                ),
            })?,
        },
        _ => Arm64Insn::Other,
    })
}

#[cfg(target_arch = "aarch64")]
fn decode_arm64_memory(
    feature: &'static str,
    instruction: &gum_sys::cs_insn,
    operands: &[gum_sys::cs_arm64_op],
) -> Result<(u32, u32, i64)> {
    let rt = operand_reg(operands, 0).ok_or_else(|| Error::UnsupportedFeature {
        feature,
        reason: format!(
            "unable to decode ART ExceptionClear memory source register at {:#x}",
            instruction.address
        ),
    })?;
    let (rn, offset) = operand_mem(operands, 1).ok_or_else(|| Error::UnsupportedFeature {
        feature,
        reason: format!(
            "unable to decode ART ExceptionClear memory operand at {:#x}",
            instruction.address
        ),
    })?;
    Ok((rt, rn, offset))
}

#[cfg(target_arch = "aarch64")]
pub(super) fn detect_jni_ids_indirection_offset(
    feature: &'static str,
    set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    let mut relocator = RawArm64Relocator::new(set_jni_id_type as u64, None);
    let mut previous = None;

    for _ in 0..20 {
        let (offset, instruction) = relocator.read_one();
        if offset == 0 || instruction.is_null() {
            return Ok(None);
        }

        let instruction = unsafe { &*instruction };
        let offset = match instruction.id {
            gum_sys::arm64_insn_ARM64_INS_CMP => previous
                .filter(|access: &Arm64MemoryAccess| access.kind == Arm64MemoryAccessKind::Load)
                .map(|access| access.offset),
            gum_sys::arm64_insn_ARM64_INS_BL => previous
                .filter(|access: &Arm64MemoryAccess| access.kind == Arm64MemoryAccessKind::Store)
                .map(|access| access.offset),
            _ => None,
        };

        if let Some(offset) = offset
            && (0x100..=0x400).contains(&offset)
        {
            return Ok(Some(offset as usize));
        }

        previous = arm64_runtime_memory_access(feature, instruction)?;
    }

    Ok(None)
}

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arm64MemoryAccessKind {
    Load,
    Store,
}

#[cfg(target_arch = "aarch64")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Arm64MemoryAccess {
    kind: Arm64MemoryAccessKind,
    offset: i64,
}

#[cfg(target_arch = "aarch64")]
fn arm64_runtime_memory_access(
    feature: &'static str,
    instruction: &gum_sys::cs_insn,
) -> Result<Option<Arm64MemoryAccess>> {
    let kind = match instruction.id {
        gum_sys::arm64_insn_ARM64_INS_LDR => Arm64MemoryAccessKind::Load,
        gum_sys::arm64_insn_ARM64_INS_STR => Arm64MemoryAccessKind::Store,
        _ => return Ok(None),
    };

    let detail = NonNull::new(instruction.detail).ok_or_else(|| Error::UnsupportedFeature {
        feature,
        reason: format!(
            "unable to decode ART Runtime::SetJniIdType instruction detail at {:#x}",
            instruction.address
        ),
    })?;
    let arm64 = unsafe { detail.as_ref().__bindgen_anon_1.arm64 };
    let operands = &arm64.operands[..arm64.op_count as usize];
    let (_, rn, offset) = decode_arm64_memory(feature, instruction, operands)?;
    if rn != gum_sys::arm64_reg_ARM64_REG_X0 {
        return Ok(None);
    }

    Ok(Some(Arm64MemoryAccess { kind, offset }))
}

#[cfg(target_arch = "aarch64")]
fn operand_reg(operands: &[gum_sys::cs_arm64_op], index: usize) -> Option<u32> {
    let operand = operands.get(index)?;
    if operand.type_ == gum_sys::arm64_op_type_ARM64_OP_REG {
        Some(unsafe { operand.__bindgen_anon_1.reg })
    } else {
        None
    }
}

#[cfg(target_arch = "aarch64")]
fn operand_imm(operands: &[gum_sys::cs_arm64_op], index: usize) -> Option<i64> {
    let operand = operands.get(index)?;
    if operand.type_ == gum_sys::arm64_op_type_ARM64_OP_IMM {
        Some(unsafe { operand.__bindgen_anon_1.imm })
    } else {
        None
    }
}

#[cfg(target_arch = "aarch64")]
fn operand_mem(operands: &[gum_sys::cs_arm64_op], index: usize) -> Option<(u32, i64)> {
    let operand = operands.get(index)?;
    if operand.type_ != gum_sys::arm64_op_type_ARM64_OP_MEM {
        return None;
    }

    let mem = unsafe { operand.__bindgen_anon_1.mem };
    Some((mem.base, mem.disp as i64))
}

#[cfg(target_arch = "aarch64")]
fn is_neutered_thread_store(_offset: i64) -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
fn is_arm64_zero_register(reg: u32) -> bool {
    reg == gum_sys::arm64_reg_ARM64_REG_XZR || reg == gum_sys::arm64_reg_ARM64_REG_WZR
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
fn put_arm64_bcond_label(writer: &Aarch64InstructionWriter, cond: u32, label: u64) {
    unsafe {
        gum_sys::gum_arm64_writer_put_b_cond_label(
            writer.raw_writer(),
            cond,
            label as *const c_void,
        )
    };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_mov_reg_reg(writer: &Aarch64InstructionWriter, dst: u32, src: u32) {
    unsafe { gum_sys::gum_arm64_writer_put_mov_reg_reg(writer.raw_writer(), dst, src) };
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
fn put_arm64_tbz_label(writer: &Aarch64InstructionWriter, reg: u32, bit: u32, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_tbz_reg_imm_label(
            writer.raw_writer(),
            reg,
            bit,
            label as *const c_void,
        )
    };
}

#[cfg(target_arch = "aarch64")]
fn put_arm64_tbnz_label(writer: &Aarch64InstructionWriter, reg: u32, bit: u32, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_tbnz_reg_imm_label(
            writer.raw_writer(),
            reg,
            bit,
            label as *const c_void,
        )
    };
}
