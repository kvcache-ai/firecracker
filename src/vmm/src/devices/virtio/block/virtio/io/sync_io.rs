// Copyright 2021 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use vm_memory::{GuestMemoryError, ReadVolatile, WriteVolatile};

use super::{AlignedBuf, DIRECT_IO_ALIGN};
use crate::vstate::memory::{Bytes, GuestAddress, GuestMemory, GuestMemoryMmap};

#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum SyncIoError {
    /// Flush: {0}
    Flush(std::io::Error),
    /// Seek: {0}
    Seek(std::io::Error),
    /// SyncAll: {0}
    SyncAll(std::io::Error),
    /// Transfer: {0}
    Transfer(GuestMemoryError),
    /// BounceBuffer: {0}
    BounceBuffer(std::io::Error),
    /// BounceBufferAlloc: failed to allocate aligned bounce buffer
    BounceBufferAlloc,
}

#[derive(Debug)]
pub struct SyncFileEngine {
    file: File,
    direct: bool,
}

// SAFETY: `File` is send and ultimately a POD.
unsafe impl Send for SyncFileEngine {}

impl SyncFileEngine {
    pub fn from_file(file: File, direct: bool) -> SyncFileEngine {
        SyncFileEngine { file, direct }
    }

    #[cfg(test)]
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Update the backing file and direct-IO flag of the engine.
    pub fn update_file(&mut self, file: File, direct: bool) {
        self.file = file;
        self.direct = direct;
    }

    /// Returns true if the guest memory address requires a bounce buffer for O_DIRECT.
    fn needs_bounce_buf(&self, addr: GuestAddress) -> bool {
        self.direct && !(addr.0 as usize).is_multiple_of(DIRECT_IO_ALIGN)
    }

    pub fn read(
        &mut self,
        offset: u64,
        mem: &GuestMemoryMmap,
        addr: GuestAddress,
        count: u32,
    ) -> Result<u32, SyncIoError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(SyncIoError::Seek)?;

        if self.needs_bounce_buf(addr) {
            // Read from disk into aligned bounce buffer, then copy to guest memory.
            let mut bounce = AlignedBuf::new(count as usize, DIRECT_IO_ALIGN)
                .ok_or(SyncIoError::BounceBufferAlloc)?;
            self.file
                .read_exact(bounce.as_mut_slice())
                .map_err(SyncIoError::BounceBuffer)?;
            mem.write_slice(bounce.as_slice(), addr)
                .map_err(SyncIoError::Transfer)?;
        } else {
            mem.get_slice(addr, count as usize)
                .and_then(|mut slice| Ok(self.file.read_exact_volatile(&mut slice)?))
                .map_err(SyncIoError::Transfer)?;
        }

        Ok(count)
    }

    pub fn write(
        &mut self,
        offset: u64,
        mem: &GuestMemoryMmap,
        addr: GuestAddress,
        count: u32,
    ) -> Result<u32, SyncIoError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(SyncIoError::Seek)?;

        if self.needs_bounce_buf(addr) {
            // Copy guest memory into aligned bounce buffer, then write to disk.
            let mut bounce = AlignedBuf::new(count as usize, DIRECT_IO_ALIGN)
                .ok_or(SyncIoError::BounceBufferAlloc)?;
            mem.read_slice(bounce.as_mut_slice(), addr)
                .map_err(SyncIoError::Transfer)?;
            self.file
                .write_all(bounce.as_slice())
                .map_err(SyncIoError::BounceBuffer)?;
        } else {
            mem.get_slice(addr, count as usize)
                .and_then(|slice| Ok(self.file.write_all_volatile(&slice)?))
                .map_err(SyncIoError::Transfer)?;
        }

        Ok(count)
    }

    pub fn flush(&mut self) -> Result<(), SyncIoError> {
        // flush() first to force any cached data out of rust buffers.
        self.file.flush().map_err(SyncIoError::Flush)?;
        // Sync data out to physical media on host.
        self.file.sync_all().map_err(SyncIoError::SyncAll)
    }
}
