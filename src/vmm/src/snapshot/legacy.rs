// Copyright 2026 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Frozen x86_64 v9.0.0 wire layout from aenv-deps@90288c397d6d3fa576f9d367d997bdeebf835f10.
//! Do not add current-layout fields here. Ordinary v9 snapshots contain no nested guest state.

use crate::arch::x86_64::vcpu::VcpuState;
use crate::device_manager::DevicesState;
use crate::persist::{MicrovmState, VmInfo};
use crate::vstate::kvm::KvmState;
use crate::vstate::vm::VmState;
use kvm_bindings::{
    CpuId, Msrs, Xsave, kvm_debugregs, kvm_lapic_state, kvm_mp_state, kvm_regs, kvm_sregs,
    kvm_vcpu_events, kvm_xcrs,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct VcpuStateV9 {
    pub cpuid: CpuId,
    pub saved_msrs: Vec<Msrs>,
    pub debug_regs: kvm_debugregs,
    pub lapic: kvm_lapic_state,
    pub mp_state: kvm_mp_state,
    pub regs: kvm_regs,
    pub sregs: kvm_sregs,
    pub vcpu_events: kvm_vcpu_events,
    pub xcrs: kvm_xcrs,
    pub xsave: Xsave,
    pub tsc_khz: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MicrovmStateV9 {
    pub vm_info: VmInfo,
    pub kvm_state: KvmState,
    pub vm_state: VmState,
    pub vcpu_states: Vec<VcpuStateV9>,
    pub device_states: DevicesState,
}

impl From<MicrovmStateV9> for MicrovmState {
    fn from(old: MicrovmStateV9) -> Self {
        Self {
            vm_info: old.vm_info,
            kvm_state: old.kvm_state,
            vm_state: old.vm_state,
            vcpu_states: old
                .vcpu_states
                .into_iter()
                .map(|old| VcpuState {
                    cpuid: old.cpuid,
                    saved_msrs: old.saved_msrs,
                    debug_regs: old.debug_regs,
                    lapic: old.lapic,
                    mp_state: old.mp_state,
                    regs: old.regs,
                    sregs: old.sregs,
                    vcpu_events: old.vcpu_events,
                    xcrs: old.xcrs,
                    xsave: old.xsave,
                    tsc_khz: old.tsc_khz,
                    nested_state: None,
                })
                .collect(),
            device_states: old.device_states,
        }
    }
}
