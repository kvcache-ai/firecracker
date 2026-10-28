# Historical snapshot fixture

snapshot-v9-x86_64.bin is an unmodified, 25,465-byte ordinary x86_64 state file
created by the official old Firecracker executable, not a current serializer.
Only VMM state is included; guest memory, disks and private AgentENV state are
not included. The isolated BusyBox guest had no credentials; MMDS is absent.

- Generator source: aenv-deps@90288c397d6d3fa576f9d367d997bdeebf835f10.
- Official archive: https://github.com/kvcache-ai/firecracker/releases/download/aenv-deps/firecracker-1.15.1-patch-v1-x86_64.tgz
- Archive SHA256: ac69af73ba5300332a9914397c8e81bdc6764711b5c190e6b8d748578bbae96b.
- Executable SHA256: 280fda256adc3093d9e14d8291b77eb3b8bab33f734a7fb96f0ab048a8aa6f51.
- Guest kernel: 6.1.175, SHA256 cd7c5a06a2026a2d8a0cddf60b41a5d76a6ac025e54439eb3ee5c9e57a60c67b.
- Fixture SHA256: 27c2e0a2b6ca8d979050def88f773082d69a653b29487c76896b445ec08c1bad.
- Intel same-host control: two vCPUs, 256 MiB RAM, one ext4 root disk and one TAP.
  Boot the old guest, save a full snapshot, restore it with that same old
  executable and verify its memory/disk witness and gateway ping before using
  the state file. The separate implementation acceptance evidence records this
  old-runtime validation and its subsequent new-runtime restore.

The embedded file keeps the original CPU and virtio state, generated MAC/IP and
isolated backing paths. The regression test reads bytes without opening those
paths, KVM, guest memory or disks. It checks field/content invariants and full
converted-state equality after saving and reloading v1000.0.0.

Run the always-enabled regression with:

```sh
cargo test --locked --target x86_64-unknown-linux-gnu -p vmm --lib snapshot::tests::test_microvm_v9_checked_in_fixture -- --exact
```

The external-fixture test remains an optional generator for separate negative
hardware fixtures; it is not required for the embedded historical regression.
