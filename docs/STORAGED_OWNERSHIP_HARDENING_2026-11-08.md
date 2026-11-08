# storaged ownership and identifier hardening — 2026-11-08

## Scope

This change hardens the existing fixed-capacity metadata/cache service. It does not implement a filesystem, block-device driver, persistent snapshots or durable storage.

## Identifier contract

Volume, mount, snapshot and package-file IDs are monotonically allocated non-zero u32 values.

- A free table slot is found before an ID is consumed.
- u32::MAX may be issued once.
- The counter then becomes zero, which is a permanent exhausted sentinel.
- No counter wraps and no identifier is reused after deletion.

## Ownership consistency

Every volume has a non-zero owner_process_id. Mounts, snapshots, cached blocks and package-file metadata retain the same owner identity. Child operations require the supplied owner to match the volume/record owner.

This is an in-service consistency check, not an authenticated security boundary. The storage crate does not have a trusted current-task token or process-table lookup. Before exposing these methods to untrusted callers, a supervisor/IPC adapter must derive the owner identity from kernel-owned caller context and validate that the process is live.

## Bounds checks

- Volume block count must be non-zero.
- start_lba + block_count must not overflow.
- Cache reads/writes must use an LBA below the volume's block count.
- Snapshot and package-file metadata cannot be created for an unknown volume.
- Owner ID zero is rejected.

## Recovery semantics

request_snapshot_recovery only checks for an owned snapshot metadata record and records a request count. It does not copy blocks, restore data or prove that a snapshot is durable.

## Required tests

The crate tests cover:

- Volume, mount, snapshot and file ID exhaustion without wraparound.
- Permanent zero-sentinel exhaustion.
- Full volume table without ID consumption.
- Zero owner, empty volume and overflowing LBA range rejection.
- Unknown-volume rejection for mount, snapshot, cache and package metadata.
- Cross-owner rejection for mount, snapshot, cache, recovery request and package deletion.
- Cache LBA range enforcement.

## Outstanding work

- Trusted caller identity from the kernel/service supervisor.
- Actual block-device read/write and flush semantics.
- Persistent filesystem and crash/power-loss recovery.
- Durable snapshot implementation and verified restoration.
