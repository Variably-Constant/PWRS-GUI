---
weight: 40
---

# Exotic pointer types (`subetha-pointers`)

`subetha-pointers` is a kit of pointer encodings. Each one is a
pure-Rust struct pairing an address with extra bits: a content
prefix, a Bloom filter, a stride, a version, a CHERI capability
bound. The address is a raw `*const T` / `*mut T`, an `Arc<T>`, or
a 64-bit word with the extra bits packed into it. The point is that
the consumer can take a useful action - skip a deref, prune a hash
bucket, branch on type, validate a bound - without going through
the data.

Every pointer here except `KTower2` / `KTower3` is an in-process
encoding: the address it carries is valid only within the
constructing process. They are not cross-process-portable and do
not travel through an MMF unchanged. `KTower2` / `KTower3` hold
region indices rather than addresses, which resolve in any process
holding the same region tables. For the cross-process siblings -
position-independent encodings that resolve in any process holding
the same region - use the `subetha-cxc` pointer family (`OffsetPtr`,
`TaggedOffsetPtr`, `SharedUmbraPointer`).

## The nine modules

| Module | Type | Payload beside the pointer |
|---|---|---|
| `umbra_pointer` | `UmbraPointer<T>` | 4-byte content prefix; short-circuit equality |
| `bloom_pointer` | `BloomPointer<T>` | 64-bit Bloom filter; probabilistic set membership |
| `cardinality_pointer` | `CardinalityPointer<T>` | log2 cardinality estimate; size-class branching |
| `kstep_pointer` | `KStepPointer<T>` | log2 stride; strided iteration |
| `k_tower_pointer` | `KTower2<T>` / `KTower3<T>` | multi-segment zone/region/offset address |
| `self_desc_pointer` | `SelfDescPointer<T>` | type ID + layout shape; dispatch over a heterogeneous container |
| `versioned_pointer` | `VersionedPointer<T>` / `HlcVersionedPointer<T>` / `VectorClockPointer<T, N>` | version counter, hybrid logical clock or vector clock; MVCC + causal ordering |
| `adaptive_cheri_pointer` | `ReadableCapability<'a, T>` / `WritableCapability<'a, T>` | runtime bounds + permissions (CHERI-style, checked in software) |
| `adaptive_rasp_batch` | `RaspBatch<T>` / `RaspBatchIndex<T>` | SoA-stored (base, length, perms); AVX2 / AVX-512F batch validation on x86 |

The structure of the crate mirrors that:

```text
subetha-pointers/
└── src/
    ├── lib.rs
    ├── umbra_pointer.rs
    ├── bloom_pointer.rs
    ├── cardinality_pointer.rs
    ├── kstep_pointer.rs
    ├── k_tower_pointer.rs
    ├── self_desc_pointer.rs
    ├── versioned_pointer.rs
    ├── adaptive_cheri_pointer.rs
    └── adaptive_rasp_batch.rs
```

## Direction signatures

Most of these pointer types expose a `SIGNATURE: AxisMask`
associated const, built via [`subetha_core::AxisMask::from_axes`],
declaring which `Axis` it engages. The actual axes are
`ContentPrefix` (Umbra,
Bloom, Cardinality), `Stride` (KStep), `Segmented` (KTower2 /
KTower3), `TypeTag` (SelfDesc), `Version` (Versioned / HLC), and
`Bounds` (the CHERI capabilities). `AxisMask::satisfies` tests one
signature against another by containment (not equality), so a
signature asking for a subset of a pointer's axes matches it;
nothing in the crates routes on these constants.

## Subsections

- [Exotic pointers](exotic-pointers/) - umbra, bloom, cardinality,
  kstep, ktower, self-desc, versioned. The seven content-prefix /
  metadata-carrying modules.
- [Bounds-check pointers](bounds-check/) - the CHERI-style
  `ReadableCapability` / `WritableCapability` pair, checked in
  software, plus the `RaspBatch<T>` SIMD-batched validator that
  runs the same (base, length, perms) checks on x86 silicon via
  AVX2 / AVX-512F.

## Stable-Rust portable

No nightly features. The pointer types are pure-Rust structs. They
build on stable Rust 1.96+, on x86_64 and aarch64, on Windows,
Linux, macOS and FreeBSD.

## See also

- [The CXC primitives catalog](../subetha-cxc/) - the cross-process
  primitives, the position-independent pointer family among them.
- [The substrate's `AxisMask`](../subetha-core/_index.md) - how the
  direction signatures are encoded.
