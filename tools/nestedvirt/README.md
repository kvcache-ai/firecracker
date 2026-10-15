# Linux 6.18 nested KVM guest

Use this x86_64 guest configuration with `v1.15.1-patch-nestedvirt` (tested at
`c7e1cceb6bf643f239ddac804a535ceb8ac93cb1`). It builds Intel and AMD KVM into
the guest kernel. This is the complete generated configuration used by the
validated Intel candidate.

`VIRTIO_MMIO_CMDLINE_DEVICES` discovers the MMIO disks described by
Firecracker's kernel command line. `X86_MPPARSE` discovers its processors and
interrupt controllers through the MP table; ACPI is disabled for this guest
configuration. Without these settings, the guest can fail to find its root disk
or run with one processor and broken interrupt behavior. These are guest
settings; the host still needs working KVM and nested VMX/SVM support.

On a Linux x86_64 build machine, install GCC, make, binutils, flex, bison,
libelf development headers, OpenSSL development headers, curl and xz. Run the
following command from this checkout, using a fresh output directory beneath the
ignored `build/` directory.

```sh
tools/nestedvirt/build-kernel.sh "$PWD/build/nestedvirt"
```

The script verifies the official Linux 6.18 source archive's SHA-256 and builds
`vmlinux` and `bzImage`. Use the uncompressed ELF
`candidates/vmlinux-6.18-agentenv-nestedvirt-x86_64` with Firecracker; the
fixture uses `kernel-build/arch/x86/boot/bzImage` for QEMU. Compiler version and
build timestamp affect the resulting binary hash; retain the generated config
and checksum alongside each deployed artifact. The script retains tool versions,
input checksums, configuration output and build logs. It fails if extracted
sources or the build directory already exist.

Build Firecracker and `cpu-template-helper` from the same checkout using the
repository's normal build tooling and the pinned Rust 1.93.0 toolchain. Put the
matching executables at
`/absolute/path/nestedvirt-work/candidates/{firecracker,cpu-template-helper}`.
The binary's inherited version string does not uniquely identify this branch;
record the Git commit and binary SHA-256 in the deployment manifest.

For a hardware smoke, install QEMU x86 system binaries, static BusyBox, cpio,
e2fsprogs, Bash and Python 3 on the Linux host. It creates a disposable 256 MiB
root disk and a real inner Linux guest; QEMU explicitly selects `accel=kvm`.

```sh
python3 tools/nestedvirt/prepare_probe.py /absolute/path/nestedvirt-work
python3 tools/nestedvirt/raw_smoke.py /absolute/path/nestedvirt-work --rounds 3
```

The three stages capture the outer VM before userspace KVM use, after QEMU has
initialized KVM with its guest stopped, and while the inner guest emits a
counter. Each stage verifies a fresh outer snapshot load and starts a new inner
guest afterward. The running stage additionally requires the old inner counter
to advance. Inspect `evidence/raw-results.json` and per-VM serial logs. Nine
PASS records are required for a three-round run; partial results are not
completion. The tool retains snapshot files for diagnosis. Use a dedicated
output directory and sufficient disk space (at least 5 GiB for this matrix).

The initialized stage checks QMP `query-kvm` and `query-status` before capture
and after restore, requiring KVM enabled/present and a stopped `prelaunch`
instance. The fixture exposes QMP only on guest loopback port 4444.
The initialized stage replaces the restored stopped `-S` inner instance before
fresh startup. It does not test continuing that same initialized inner instance.

PASS is recorded after the restored Firecracker process stops successfully.
The `Nested KVM tools` GitHub workflow checks Python and shell syntax on hosted
runners. The nine-case hardware matrix must also run on a dedicated nested-KVM
host with matching binaries and kernel; syntax checks do not exercise KVM.

Measured coverage for the integration candidate is Intel VMX, one host, two
outer vCPUs and 512 MiB outer memory. AMD KVM is built in but needs its own
hardware run. Snapshot compatibility with older Firecracker branches is not
established; deploy this candidate into separate state and rebuild templates.
