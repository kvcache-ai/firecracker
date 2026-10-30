# Copyright 2026 Amazon.com, Inc. or its affiliates. All Rights Reserved.
# SPDX-License-Identifier: Apache-2.0
"""Explicit clear is a post-commit operation, independent of memory export."""

import os
import time
from dataclasses import replace
from pathlib import Path

import pytest

from host_tools.cargo_build import gcc_compile


@pytest.fixture
def dirty_memory_helper(tmp_path):
    """Compile the ordinary-page witness for the native guest architecture."""
    binary = tmp_path / "dirty_memory_helper"
    gcc_compile("host_tools/dirty_memory_helper.c", binary)
    return binary


def wait_for_status(vm, value):
    """Wait until the guest has completed an explicit page-writing command."""
    for _ in range(100):
        result = vm.ssh.run("cat /tmp/dirty-pages.status")
        if result.returncode == 0 and result.stdout.strip() == str(value):
            return
        time.sleep(0.05)
    raise AssertionError(f"Guest did not complete dirty-page command {value}")


def command(vm, value):
    """Update pages while running, and wait for guest completion."""
    vm.ssh.check_output(f"echo {value} > /tmp/dirty-pages.command")
    wait_for_status(vm, value)


def includes(ranges, image_offset):
    """Check an image offset, never a guest physical address."""
    return any(
        entry["image_offset"] <= image_offset < entry["image_offset"] + entry["length"]
        for entry in ranges["ranges"]
    )


def controlled_offsets(patches, page_size):
    """Locate the unique ordinary-page witnesses in the exported image layout."""
    offsets = []
    for byte in [0x11, 0x22, 0x33]:
        matches = []
        for offset, data in patches:
            pattern = bytes([byte]) * page_size
            position = data.find(pattern)
            while position != -1:
                if position % page_size == 0:
                    matches.append(offset + position)
                position = data.find(pattern, position + 1)
        assert len(matches) == 1, f"Ambiguous controlled page {byte}: {matches}"
        offsets.append(matches[0])
    return offsets


def read_ranges(vm, ranges):
    """Read exact ranges from the paused Firecracker process, checking short reads."""
    patches = []
    descriptor = os.open(f"/proc/{vm.firecracker_pid}/mem", os.O_RDONLY)
    try:
        for entry in ranges:
            data = os.pread(descriptor, entry["length"], entry["base_host_virt_addr"])
            assert len(data) == entry["length"], "Short external memory read"
            patches.append((entry["image_offset"], data))
    finally:
        os.close(descriptor)
    return patches


def commit_image(path, patches, *, size=None):
    """Test external commit: flush all image bytes, then sync the directory."""
    with path.open("w+b" if size is not None else "r+b") as image:
        if size is not None:
            image.truncate(size)
        for offset, data in patches:
            image.seek(offset)
            assert image.write(data) == len(data)
        image.flush()
        os.fsync(image.fileno())
    directory = os.open(path.parent, os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def capture_state(vm, name):
    """State-only capture performs device preparation without resetting bitmaps."""
    vm.pause()
    vm.api.snapshot_create.put(snapshot_path=name)
    return Path(vm.chroot()) / name


def test_clear_dirty_memory_without_tracking(uvm_plain):
    """Untracked CLEAR succeeds without changing resident memory or VM state."""
    vm = uvm_plain
    vm.spawn()
    vm.basic_config(mem_size_mib=128, track_dirty_pages=False)
    vm.add_net_iface()
    vm.start()
    with pytest.raises(RuntimeError, match="paused"):
        vm.api.clear_dirty_memory.put()
    capture_state(vm, "untracked.state")
    before = vm.api.dirty_memory_ranges.get().json()
    assert before["ranges"]
    contents = read_ranges(vm, before["ranges"])
    for _ in range(2):
        response = vm.api.clear_dirty_memory.put()
        assert response.status_code == 204 and response.content == b""
        assert vm.api.describe.get().json()["state"] == "Paused"
        after = vm.api.dirty_memory_ranges.get().json()
        assert after == before
        assert read_ranges(vm, before["ranges"]) == contents
    vm.resume()
    vm.ssh.check_output("true")


def test_clear_dirty_memory_incremental_restore(
    uvm_plain, microvm_factory, dirty_memory_helper, tmp_path
):
    """Clear KVM-only bits, export A, then restore B's native Diff on S1."""
    vm = uvm_plain
    vm.spawn()
    vm.basic_config(mem_size_mib=128, track_dirty_pages=True)
    vm.add_net_iface()
    vm.start()
    vm.ssh.scp_put(dirty_memory_helper, "/tmp/dirty_memory_helper")
    vm.ssh.check_output(
        "nohup /tmp/dirty_memory_helper >/tmp/dirty-pages.log 2>&1 </dev/null &"
    )
    wait_for_status(vm, 0)
    # A full memory read does not consume the KVM dirty log.
    state = capture_state(vm, "s0.state")
    mappings = vm.api.guest_memory_regions.get().json()
    ranges = [
        {
            "base_host_virt_addr": m["base_host_virt_addr"],
            "image_offset": m["offset"],
            "length": m["size"],
        }
        for m in mappings
    ]
    baseline = read_ranges(vm, ranges)
    page_size = mappings[0]["page_size"]
    offsets = controlled_offsets(baseline, page_size)
    image = tmp_path / "s1.mem"
    commit_image(image, baseline, size=sum(m["size"] for m in mappings))
    # No dirty GET or native memory snapshot before this CLEAR.
    response = vm.api.clear_dirty_memory.put()
    assert response.status_code == 204 and response.content == b""
    after = vm.api.dirty_memory_ranges.get().json()
    assert all(not includes(after, offset) for offset in offsets)

    vm.resume()
    command(vm, 1)
    state = capture_state(vm, "s1.state")
    dirty = vm.api.dirty_memory_ranges.get().json()
    assert includes(dirty, offsets[0])
    assert not includes(dirty, offsets[1]) and not includes(dirty, offsets[2])
    commit_image(image, read_ranges(vm, dirty["ranges"]))
    vm.api.clear_dirty_memory.put()
    vm.resume()
    command(vm, 2)
    # Do not GET before Diff: native capture must observe KVM's new writes.
    delta = vm.snapshot_diff(mem_path="delta.mem", vmstate_path="s2.state")
    with delta.mem.open("rb") as memory:
        memory.seek(offsets[0])
        assert memory.read(page_size) == bytes(page_size)
    correct = delta.rebase_snapshot(
        replace(delta, mem=image, vmstate=state), use_snapshot_editor=True
    )
    vm.kill()
    restored = microvm_factory.build_from_snapshot(correct)
    command(restored, 4)  # Verify A, B and explicit zero bytes inside the guest.
