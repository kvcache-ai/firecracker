#!/usr/bin/env bash
# Copyright 2026 AgentENV contributors.
# SPDX-License-Identifier: Apache-2.0
# Build the Linux 6.18 guest kernel with the verified configuration.
set -euo pipefail
work=${1:?build-kernel.sh OUTPUT_DIRECTORY}
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
mkdir -p "$work"
work=$(cd "$work" && pwd)
cd "$work"
mkdir -p intermediate
export TMPDIR="$work/intermediate"
version=6.18
checksum=9106a4605da9e31ff17659d958782b815f9591ab308d03b0ee21aad6c7dced4b
if [[ ! -f linux-$version.tar.xz ]]; then
    curl --fail --location --retry 3 -o linux-$version.tar.xz "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$version.tar.xz"
fi
printf '%s  linux-%s.tar.xz\n' "$checksum" "$version" | sha256sum -c -
test ! -e "linux-$version"
test ! -e kernel-build
tar -xf "linux-$version.tar.xz"
mkdir kernel-build
sha256sum "$repo/resources/guest_configs/microvm-kernel-nestedvirt-x86_64-6.18.config" "$repo/tools/nestedvirt/build-kernel.sh" > build-inputs.sha256
{ uname -m; gcc --version; ld --version; make --version; } > build-tools.txt
cp "$repo/resources/guest_configs/microvm-kernel-nestedvirt-x86_64-6.18.config" kernel-build/.config
make -C "linux-$version" O="$work/kernel-build" olddefconfig 2>&1 | tee configure.log
for symbol in KVM KVM_INTEL KVM_AMD X86_MPPARSE DAMON_RECLAIM PAGE_REPORTING DEVTMPFS DEVTMPFS_MOUNT VIRTIO_MMIO VIRTIO_MMIO_CMDLINE_DEVICES VIRTIO_BLK VIRTIO_NET VIRTIO_VSOCKETS EXT4_FS; do
    grep -qx "CONFIG_${symbol}=y" kernel-build/.config || { echo "Missing built-in $symbol" >&2; exit 1; }
done
make -C "linux-$version" O="$work/kernel-build" -j"${JOBS:-8}" vmlinux bzImage 2>&1 | tee build.log
mkdir -p candidates
cp kernel-build/vmlinux candidates/vmlinux-6.18-agentenv-nestedvirt-x86_64
cp kernel-build/.config candidates/linux-6.18-x86_64.config
(cd candidates && sha256sum vmlinux-6.18-agentenv-nestedvirt-x86_64 linux-6.18-x86_64.config > kernel-SHA256SUMS)
file candidates/vmlinux-6.18-agentenv-nestedvirt-x86_64
