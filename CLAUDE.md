# CLAUDE.md - Recstrap

## STOP. READ. THEN ACT.

Before modifying this crate, read `src/main.rs` to understand the installation flow.

---

## What is recstrap?

LevitateOS installer. Extracts the squashfs from the live ISO to disk and configures the bootloader. Equivalent to Arch's `archinstall`.

## Development

```bash
cargo build --release    # LTO + strip enabled
cargo clippy
```

## Key Rules

1. **Don't skip confirmation** - Always confirm before erasing disks
2. **Handle NVMe naming** - `/dev/nvme0n1p1` vs `/dev/sda1`
3. **Use UUIDs in fstab** - Never use device paths
4. **Keep it simple** - This runs in the live ISO environment

## Installation Steps

1. Partition disk (GPT)
2. Format partitions (FAT32 + ext4)
3. Mount partitions
4. Extract squashfs
5. Generate fstab
6. Install systemd-boot
7. Set root password
8. Unmount

## Testing

Test in QEMU with a virtual disk, not on real hardware during development.
