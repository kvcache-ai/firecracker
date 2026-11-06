// Copyright 2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Provides serialization and deserialization facilities and implements a persistent storage
//! format for Firecracker state snapshots.
//!
//! The `Snapshot` API manages serialization and deserialization of collections of objects
//! that implement the `serde` `Serialize`, `Deserialize` trait. Currently, we use
//! [`bitcode`](https://docs.rs/bitcode/latest/bitcode/) for performing the serialization.
//!
//! The snapshot format uses the following layout:
//!
//!  |-----------------------------|
//!  |       64 bit magic_id       |
//!  |-----------------------------|
//!  |       version string        |
//!  |-----------------------------|
//!  |            State            |
//!  |-----------------------------|
//!  |        optional CRC64       |
//!  |-----------------------------|
//!
//!
//! The snapshot format uses a version value in the form of `MAJOR.MINOR.PATCH`. The version is
//! provided by the library clients (it is not tied to this crate).
pub mod crc;
#[cfg(target_arch = "x86_64")]
mod legacy;
mod persist;
use std::fmt::Debug;
use std::io::{Read, Write};

use crc64::crc64;
use semver::Version;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::persist::SNAPSHOT_VERSION;
use crate::snapshot::crc::CRC64Writer;
pub use crate::snapshot::persist::Persist;

#[cfg(target_arch = "x86_64")]
const SNAPSHOT_MAGIC_ID: u64 = 0x0710_1984_8664_0000u64;

#[cfg(target_arch = "aarch64")]
const SNAPSHOT_MAGIC_ID: u64 = 0x0710_1984_AAAA_0000u64;

/// Maximum size in bytes for snapshot deserialization to prevent DOS attacks.
/// Snapshots contain VM state which can be large, but we set a reasonable upper bound.
/// This limit is 10MB which should be sufficient for any legitimate snapshot.
const SNAPSHOT_DESERIALIZATION_BYTES_LIMIT: usize = 10_000_000;

/// Error definitions for the Snapshot API.
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum SnapshotError {
    /// CRC64 validation failed
    Crc64,
    /// Invalid data version: {0}
    InvalidFormatVersion(Version),
    /// Magic value does not match arch: {0}
    InvalidMagic(u64),
    /// An error occured during bitcode serialization: {0}
    Bitcode(#[from] bitcode::Error),
    /// IO Error: {0}
    Io(#[from] std::io::Error),
    /// Snapshot size exceeds limit of {0} bytes
    SizeLimitExceeded(usize),
}

fn serialize<S: Serialize, W: Write>(data: &S, write: &mut W) -> Result<(), SnapshotError> {
    let encoded = bitcode::serialize(data)?;
    write.write_all(&encoded).map_err(SnapshotError::Io)
}

/// Firecracker snapshot header
#[derive(Debug, Serialize, Deserialize)]
struct SnapshotHdr {
    /// magic value
    magic: u64,
    /// Snapshot data version
    version: Version,
}

/// Assumes the raw bytes stream read from the given [`Read`] instance is a snapshot file,
/// and returns the version of it.
pub fn get_format_version<R: Read>(reader: &mut R) -> Result<Version, SnapshotError> {
    Ok(load_microvm_snapshot(reader)?.format_version)
}

/// Decoded VM state and its original file format. This type cannot be serialized:
/// use `Snapshot::new(data)` to write the current layout with the current version.
#[derive(Debug)]
pub struct LoadedMicrovmSnapshot {
    /// Original file format, before in-memory conversion.
    pub format_version: Version,
    /// State converted to the current runtime layout.
    pub data: crate::persist::MicrovmState,
}

/// Load the explicitly supported VM wire layouts with size, CRC, architecture and version checks.
pub fn load_microvm_snapshot<R: Read>(
    reader: &mut R,
) -> Result<LoadedMicrovmSnapshot, SnapshotError> {
    #[cfg(target_arch = "aarch64")]
    {
        let snapshot = Snapshot::<crate::persist::MicrovmState>::load(reader)?;
        Ok(LoadedMicrovmSnapshot {
            format_version: snapshot.header.version,
            data: snapshot.data,
        })
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut buf = Vec::new();
        reader
            .take((SNAPSHOT_DESERIALIZATION_BYTES_LIMIT + 1) as u64)
            .read_to_end(&mut buf)?;
        if buf.len() > SNAPSHOT_DESERIALIZATION_BYTES_LIMIT {
            return Err(SnapshotError::SizeLimitExceeded(
                SNAPSHOT_DESERIALIZATION_BYTES_LIMIT,
            ));
        }
        if buf.len() < 8 {
            return Err(SnapshotError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "File too short to contain CRC",
            )));
        }
        if crc64(0, &buf) != 0 {
            return Err(SnapshotError::Crc64);
        }
        let data_buf = &buf[..buf.len() - 8];
        match bitcode::deserialize::<Snapshot<crate::persist::MicrovmState>>(data_buf) {
            Ok(snapshot) if snapshot.header.version != Version::new(9, 0, 0) => {
                validate_microvm_header(&snapshot.header, &SNAPSHOT_VERSION)?;
                Ok(LoadedMicrovmSnapshot {
                    format_version: snapshot.header.version,
                    data: snapshot.data,
                })
            }
            _ => {
                // Empty vCPU vectors can match both schemas. Version 9 always
                // selects the frozen wire type, even if current decoding succeeds.
                let old: Snapshot<legacy::MicrovmStateV9> = bitcode::deserialize(data_buf)?;
                validate_microvm_header(&old.header, &Version::new(9, 0, 0))?;
                Ok(LoadedMicrovmSnapshot {
                    format_version: old.header.version,
                    data: old.data.into(),
                })
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn validate_microvm_header(header: &SnapshotHdr, expected: &Version) -> Result<(), SnapshotError> {
    if header.magic != SNAPSHOT_MAGIC_ID {
        return Err(SnapshotError::InvalidMagic(header.magic));
    }
    if &header.version != expected {
        return Err(SnapshotError::InvalidFormatVersion(header.version.clone()));
    }
    Ok(())
}

/// Firecracker snapshot type
///
/// A type used to store and load Firecracker snapshots of a particular version
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot<Data> {
    header: SnapshotHdr,
    /// The data stored int his [`Snapshot`]
    pub data: Data,
}

impl<Data> Snapshot<Data> {
    /// Constructs a new snapshot with the given `data`.
    pub fn new(data: Data) -> Self {
        Self {
            header: SnapshotHdr {
                magic: SNAPSHOT_MAGIC_ID,
                version: SNAPSHOT_VERSION.clone(),
            },
            data,
        }
    }

    /// Gets the version of this snapshot
    pub fn version(&self) -> &Version {
        &self.header.version
    }
}

impl<Data: DeserializeOwned> Snapshot<Data> {
    pub(crate) fn load_without_crc_check(buf: &[u8]) -> Result<Self, SnapshotError> {
        // Check size limit to prevent DOS attacks
        if buf.len() > SNAPSHOT_DESERIALIZATION_BYTES_LIMIT {
            return Err(SnapshotError::SizeLimitExceeded(
                SNAPSHOT_DESERIALIZATION_BYTES_LIMIT,
            ));
        }

        let snapshot: Self = bitcode::deserialize(buf)?;

        // Validate the header
        if snapshot.header.magic != SNAPSHOT_MAGIC_ID {
            return Err(SnapshotError::InvalidMagic(snapshot.header.magic));
        }

        let supported_version = snapshot.header.version.major == SNAPSHOT_VERSION.major
            && snapshot.header.version.minor <= SNAPSHOT_VERSION.minor;
        // ARM's wire layout is unchanged from v9; only x86 added nested vCPU state.
        #[cfg(target_arch = "aarch64")]
        let supported_version =
            supported_version || snapshot.header.version == Version::new(9, 0, 0);
        if !supported_version {
            return Err(SnapshotError::InvalidFormatVersion(
                snapshot.header.version.clone(),
            ));
        }

        Ok(snapshot)
    }

    /// Loads a snapshot from the given [`Read`] instance, performing all validations
    /// (CRC, snapshot magic value, snapshot version).
    pub fn load<R: Read>(reader: &mut R) -> Result<Self, SnapshotError> {
        // Check size limit before reading the full file to prevent DOS attacks
        let mut buf = Vec::new();
        let bytes_read = reader
            .take((SNAPSHOT_DESERIALIZATION_BYTES_LIMIT + 1) as u64)
            .read_to_end(&mut buf)?;

        if bytes_read > SNAPSHOT_DESERIALIZATION_BYTES_LIMIT {
            return Err(SnapshotError::SizeLimitExceeded(
                SNAPSHOT_DESERIALIZATION_BYTES_LIMIT,
            ));
        }

        // The last 8 bytes are the CRC, so we need to separate them
        if buf.len() < 8 {
            return Err(SnapshotError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "File too short to contain CRC",
            )));
        }

        let (data_buf, _crc_buf) = buf.split_at(buf.len() - 8);
        let snapshot = Self::load_without_crc_check(data_buf)?;

        let computed_checksum = crc64(0, buf.as_slice());
        // When we read the entire file, we also read the checksum into the buffer. The CRC has the
        // property that crc(0, buf.as_slice()) == 0 iff the last 8 bytes of buf are the checksum
        // of all the preceeding bytes, and this is the property we are using here.
        if computed_checksum != 0 {
            return Err(SnapshotError::Crc64);
        }
        Ok(snapshot)
    }
}

impl<Data: Serialize> Snapshot<Data> {
    /// Saves `self` to the given [`Write`] instance, computing the CRC of the written data,
    /// and then writing the CRC into the `Write` instance, too.
    pub fn save<W: Write>(&self, writer: &mut W) -> Result<(), SnapshotError> {
        let mut crc_writer = CRC64Writer::new(writer);
        serialize(self, &mut crc_writer)?;
        // Write the CRC as raw bytes, not bitcode-serialized
        crc_writer
            .writer
            .write_all(&crc_writer.checksum().to_le_bytes())
            .map_err(SnapshotError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::MicrovmState;

    #[cfg(target_arch = "x86_64")]
    fn old_snapshot() -> Snapshot<legacy::MicrovmStateV9> {
        let old = crate::arch::x86_64::vcpu::VcpuState::default();
        Snapshot {
            header: SnapshotHdr {
                magic: SNAPSHOT_MAGIC_ID,
                version: Version::new(9, 0, 0),
            },
            data: legacy::MicrovmStateV9 {
                vm_info: Default::default(),
                kvm_state: Default::default(),
                vm_state: Default::default(),
                device_states: Default::default(),
                vcpu_states: vec![legacy::VcpuStateV9 {
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
                }],
            },
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_microvm_v9_conversion_and_current_output() {
        let mut bytes = Vec::new();
        old_snapshot().save(&mut bytes).unwrap();
        let loaded = load_microvm_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(loaded.format_version, Version::new(9, 0, 0));
        assert_eq!(
            get_format_version(&mut bytes.as_slice()).unwrap(),
            loaded.format_version
        );
        assert_eq!(loaded.data.vcpu_states.len(), 1);
        assert!(loaded.data.vcpu_states[0].nested_state.is_none());
        assert_eq!(loaded.data.vcpu_states[0].tsc_khz, Some(0));
        let mut upgraded = Vec::new();
        Snapshot::new(loaded.data).save(&mut upgraded).unwrap();
        let loaded = load_microvm_snapshot(&mut upgraded.as_slice()).unwrap();
        assert_eq!(loaded.format_version, SNAPSHOT_VERSION);
        assert!(loaded.data.vcpu_states[0].nested_state.is_none());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_microvm_v9_rejects_invalid_files() {
        let mut old = old_snapshot();
        for version in [
            Version::new(9, 1, 0),
            Version::new(9, 0, 1),
            Version::new(10, 0, 0),
            SNAPSHOT_VERSION,
        ] {
            old.header.version = version;
            let mut bytes = Vec::new();
            old.save(&mut bytes).unwrap();
            assert!(matches!(
                load_microvm_snapshot(&mut bytes.as_slice()),
                Err(SnapshotError::InvalidFormatVersion(_))
            ));
        }
        old.header.version = Version::new(9, 0, 0);
        old.header.magic = 0x0710_1984_AAAA_0000;
        let mut bytes = Vec::new();
        old.save(&mut bytes).unwrap();
        assert!(matches!(
            load_microvm_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::InvalidMagic(_))
        ));
        old.header.magic = SNAPSHOT_MAGIC_ID;
        bytes.clear();
        old.save(&mut bytes).unwrap();
        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(matches!(
            load_microvm_snapshot(&mut corrupt.as_slice()),
            Err(SnapshotError::Crc64)
        ));
        for length in [0, 7, bytes.len() / 2, bytes.len() - 1] {
            load_microvm_snapshot(&mut &bytes[..length]).unwrap_err();
        }
        let oversized = vec![0; SNAPSHOT_DESERIALIZATION_BYTES_LIMIT + 1];
        assert!(matches!(
            load_microvm_snapshot(&mut oversized.as_slice()),
            Err(SnapshotError::SizeLimitExceeded(_))
        ));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_microvm_v9_checked_in_fixture() {
        // Produced by the pinned old executable, never by the current serializer.
        let bytes = include_bytes!("../../testdata/snapshot-v9-x86_64.bin");
        assert_eq!(bytes.len(), 25_465);
        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
        let digest = digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "27c2e0a2b6ca8d979050def88f773082d69a653b29487c76896b445ec08c1bad"
        );
        let loaded = load_microvm_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(loaded.format_version, Version::new(9, 0, 0));
        assert_eq!(
            get_format_version(&mut bytes.as_slice()).unwrap(),
            loaded.format_version
        );
        assert_eq!(loaded.data.vm_info.mem_size_mib, 256);
        assert_eq!(loaded.data.vcpu_states.len(), 2);
        for state in &loaded.data.vcpu_states {
            assert!(!state.cpuid.as_slice().is_empty());
            assert!(state.tsc_khz.is_some());
            assert!(state.nested_state.is_none());
        }
        let expected = serde_json::to_value(&loaded.data).unwrap();
        let mut upgraded = Vec::new();
        Snapshot::new(loaded.data).save(&mut upgraded).unwrap();
        let restored = load_microvm_snapshot(&mut upgraded.as_slice()).unwrap();
        assert_eq!(restored.format_version, Version::new(1000, 0, 0));
        assert_eq!(serde_json::to_value(&restored.data).unwrap(), expected);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "exports negative fixture files for external hardware tests"]
    fn test_export_microvm_v9_negative_fixtures() {
        let path = std::env::var("FC_V9_FIXTURE")
            .expect("set FC_V9_FIXTURE to an authentic v9 state file");
        let bytes = std::fs::read(path).unwrap();
        let loaded = load_microvm_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(loaded.format_version, Version::new(9, 0, 0));
        assert!(!loaded.data.vcpu_states.is_empty());
        assert!(
            loaded
                .data
                .vcpu_states
                .iter()
                .all(|state| state.nested_state.is_none())
        );
        // Emit valid-CRC negative fixtures using the historical state's exact layout.
        // The actual VMM and both inspection tools must reject these files, too.
        let directory = std::env::var("FC_V9_NEGATIVE_DIR").expect("set FC_V9_NEGATIVE_DIR");
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        let mut old: Snapshot<legacy::MicrovmStateV9> =
            bitcode::deserialize(&bytes[..bytes.len() - 8]).unwrap();
        old.header.version = Version::new(9, 1, 0);
        old.save(&mut std::fs::File::create(directory.join("unknown-version.bin")).unwrap())
            .unwrap();
        old.header.version = Version::new(9, 0, 0);
        old.header.magic = 0x0710_1984_AAAA_0000;
        old.save(&mut std::fs::File::create(directory.join("wrong-architecture.bin")).unwrap())
            .unwrap();
        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        std::fs::write(directory.join("bad-crc.bin"), corrupt).unwrap();
        std::fs::write(directory.join("truncated.bin"), &bytes[..bytes.len() / 2]).unwrap();
        for entry in std::fs::read_dir(directory).unwrap() {
            let negative = std::fs::read(entry.unwrap().path()).unwrap();
            load_microvm_snapshot(&mut negative.as_slice()).unwrap_err();
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_arm_v9_restore_and_current_output() {
        let mut old = Snapshot::new(MicrovmState::default());
        old.header.version = Version::new(9, 0, 0);
        let mut bytes = Vec::new();
        old.save(&mut bytes).unwrap();

        let loaded = load_microvm_snapshot(&mut bytes.as_slice()).unwrap();
        assert_eq!(loaded.format_version, Version::new(9, 0, 0));
        assert_eq!(
            get_format_version(&mut bytes.as_slice()).unwrap(),
            loaded.format_version
        );
        let expected = serde_json::to_value(&loaded.data).unwrap();
        let mut upgraded = Vec::new();
        Snapshot::new(loaded.data).save(&mut upgraded).unwrap();
        let restored = load_microvm_snapshot(&mut upgraded.as_slice()).unwrap();
        assert_eq!(restored.format_version, Version::new(1000, 0, 0));
        assert_eq!(serde_json::to_value(&restored.data).unwrap(), expected);

        // Accepting v9 must not bypass the existing integrity or architecture checks.
        *bytes.last_mut().unwrap() ^= 1;
        assert!(matches!(
            load_microvm_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::Crc64)
        ));
        old.header.magic = 0x0710_1984_8664_0000;
        bytes.clear();
        old.save(&mut bytes).unwrap();
        assert!(matches!(
            load_microvm_snapshot(&mut bytes.as_slice()),
            Err(SnapshotError::InvalidMagic(_))
        ));
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_arm_rejects_unknown_legacy_versions() {
        for version in [
            Version::new(8, 0, 0),
            Version::new(9, 0, 1),
            Version::new(9, 1, 0),
            Version::new(10, 0, 0),
        ] {
            let mut snapshot = Snapshot::new(MicrovmState::default());
            snapshot.header.version = version;
            let mut bytes = Vec::new();
            snapshot.save(&mut bytes).unwrap();
            assert!(matches!(
                load_microvm_snapshot(&mut bytes.as_slice()),
                Err(SnapshotError::InvalidFormatVersion(_))
            ));
        }
    }

    #[test]
    fn test_snapshot_restore() {
        let state = MicrovmState::default();
        let mut buf = Vec::new();

        Snapshot::new(state).save(&mut buf).unwrap();
        Snapshot::<MicrovmState>::load(&mut buf.as_slice()).unwrap();
    }

    #[test]
    fn test_parse_version_from_file() {
        use crate::persist::MicrovmState;
        let snapshot = Snapshot::new(MicrovmState::default());

        // Use a Vec<u8> that can grow as needed
        let mut snapshot_data = Vec::new();
        snapshot.save(&mut snapshot_data).unwrap();

        // Debug: print the length to understand what's happening
        println!("Snapshot data length: {}", snapshot_data.len());

        assert_eq!(
            get_format_version(&mut std::io::Cursor::new(&snapshot_data)).unwrap(),
            SNAPSHOT_VERSION
        );
    }

    #[test]
    fn test_bad_reader() {
        #[derive(Debug)]
        struct BadReader;

        impl Read for BadReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::InvalidInput.into())
            }
        }

        let mut reader = BadReader {};

        assert!(
            matches!(Snapshot::<()>::load(&mut reader), Err(SnapshotError::Io(inner)) if inner.kind() == std::io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn test_bad_magic() {
        // Create a snapshot with corrupted magic and serialize it properly
        let mut bad_snapshot = Snapshot::new(());
        bad_snapshot.header.magic = 0xDEADBEEF;

        // Serialize the bad snapshot (without CRC for load_without_crc_check)
        let corrupted_data = bitcode::serialize(&bad_snapshot).unwrap();

        assert!(matches!(
            Snapshot::<()>::load_without_crc_check(&corrupted_data),
            Err(SnapshotError::InvalidMagic(_))
        ));
    }

    #[test]
    fn test_bad_crc() {
        let snapshot = Snapshot::new(());

        // Use a Vec<u8> that can grow as needed
        let mut valid_data = Vec::new();
        snapshot.save(&mut valid_data).unwrap();

        // Corrupt the CRC by changing the last 8 bytes (where CRC is stored)
        if valid_data.len() >= 8 {
            for i in (valid_data.len() - 8)..valid_data.len() {
                valid_data[i] ^= 0xFF; // Corrupt the CRC by flipping bits
            }
        }

        assert!(matches!(
            Snapshot::<()>::load(&mut std::io::Cursor::new(&valid_data)),
            Err(SnapshotError::Crc64)
        ));
    }

    #[test]
    fn test_bad_version() {
        // Different major version: shouldn't work
        let mut bad_snapshot = Snapshot::new(());
        bad_snapshot.header.version.major = SNAPSHOT_VERSION.major + 1;
        let data = bitcode::serialize(&bad_snapshot).unwrap();

        assert!(matches!(
            Snapshot::<()>::load_without_crc_check(&data),
            Err(SnapshotError::InvalidFormatVersion(v)) if v.major == SNAPSHOT_VERSION.major + 1
        ));

        //  minor > SNAPSHOT_VERSION.minor: shouldn't work
        let mut bad_snapshot = Snapshot::new(());
        bad_snapshot.header.version.minor = SNAPSHOT_VERSION.minor + 1;
        let data = bitcode::serialize(&bad_snapshot).unwrap();
        assert!(matches!(
            Snapshot::<()>::load_without_crc_check(&data),
            Err(SnapshotError::InvalidFormatVersion(v)) if v.minor == SNAPSHOT_VERSION.minor + 1
        ));

        // But we can support minor versions smaller or equal to ours. We also support
        // all patch versions within our supported major.minor version.
        let snapshot = Snapshot::new(());
        let data = bitcode::serialize(&snapshot).unwrap();
        Snapshot::<()>::load_without_crc_check(&data).unwrap();

        if SNAPSHOT_VERSION.minor != 0 {
            let mut snapshot = Snapshot::new(());
            snapshot.header.version.minor = SNAPSHOT_VERSION.minor - 1;
            let data = bitcode::serialize(&snapshot).unwrap();
            Snapshot::<()>::load_without_crc_check(&data).unwrap();
        }

        let mut snapshot = Snapshot::new(());
        snapshot.header.version.patch = 0;
        let data = bitcode::serialize(&snapshot).unwrap();
        Snapshot::<()>::load_without_crc_check(&data).unwrap();

        let mut snapshot = Snapshot::new(());
        snapshot.header.version.patch = SNAPSHOT_VERSION.patch + 1;
        let data = bitcode::serialize(&snapshot).unwrap();
        Snapshot::<()>::load_without_crc_check(&data).unwrap();

        let mut snapshot = Snapshot::new(());
        snapshot.header.version.patch = 1024;
        let data = bitcode::serialize(&snapshot).unwrap();
        Snapshot::<()>::load_without_crc_check(&data).unwrap();
    }
}

