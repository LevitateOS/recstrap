# CLAUDE.md - Recstrap

## STOP. READ. THEN ACT.

Before modifying this crate, read `src/main.rs` to understand the extraction flow.

---

## What is recstrap?

LevitateOS system extractor. **Like pacstrap, NOT like archinstall.**

Extracts the squashfs from the live ISO to a target directory. That's it.
User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).

## Development

```bash
cargo build --release    # LTO + strip enabled
cargo clippy
```

## Key Rules

1. **recstrap = pacstrap** - Just extract, nothing else
2. **Keep it simple** - ~50 lines, one job
3. **No automation** - User does manual install (like Arch)

## What recstrap does

```bash
recstrap /mnt                    # Extract squashfs to /mnt
recstrap /mnt --squashfs /path   # Custom squashfs location
```

## What recstrap does NOT do

- Partitioning (user runs fdisk/parted)
- Formatting (user runs mkfs)
- Mounting (user runs mount)
- fstab generation (user runs genfstab)
- Bootloader (user runs bootctl)
- Password setting (user runs passwd)
- User creation (user runs useradd)

## Testing

Test in QEMU with a virtual disk, not on real hardware during development.
