# CLAUDE.md - recstrap

## What is recstrap?

LevitateOS system extractor. **Like pacstrap, NOT like archinstall.**

Extracts rootfs (EROFS or squashfs) from live ISO to target directory. That's it. User does everything else manually.

## What Belongs Here

- Rootfs extraction logic (EROFS and squashfs)
- Pre-flight validation (root, paths, space, magic bytes)
- Error codes and cheat-aware checks

## What Does NOT Belong Here

| Don't put here | Put it in |
|----------------|-----------|
| Fstab generation | `tools/recfstab/` |
| Chroot setup | `tools/recchroot/` |
| Partitioning/formatting | User does manually |
| Bootloader installation | User does manually |

## Commands

```bash
cargo build --release    # LTO + strip enabled
cargo test
cargo clippy
```

## Usage

```bash
recstrap /mnt                    # Extract rootfs to /mnt (auto-detect format)
recstrap /mnt --rootfs /path     # Custom rootfs location (EROFS or squashfs)
recstrap /mnt --squashfs /path   # Alias for --rootfs (backwards compat)
recstrap /mnt --force            # Override non-empty/non-mount-point
recstrap /mnt --check            # Pre-flight validation only
```

## Error Codes

| Code | Exit | Description |
|------|------|-------------|
| E001 | 1 | Target does not exist |
| E002 | 2 | Target is not a directory |
| E003 | 3 | Target not writable |
| E004 | 4 | Rootfs not found |
| E005 | 5 | Extraction failed |
| E006 | 6 | Verification failed |
| E007 | 7 | Required tool not installed |
| E008 | 8 | Must run as root |
| E009 | 9 | Target not empty |
| E010 | 10 | Protected system path |
| E011 | 11 | Not a mount point |
| E012 | 12 | Insufficient space |
| E013 | 13 | Rootfs is not a file |
| E014 | 14 | Rootfs not readable |
| E015 | 15 | Rootfs inside target |
| E016 | 16 | Invalid rootfs format (bad magic) |
| E017 | 17 | EROFS not supported by kernel |

## Protected Paths (blocked even with --force)

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Rootfs Format Detection

- `.erofs` extension → EROFS (mount + cp -aT)
- `.squashfs` extension → squashfs (unsquashfs)
- Unknown → assumes squashfs for backwards compatibility

Magic bytes are validated before extraction:
- EROFS: `0xe0f5e1e2` at offset 1024
- Squashfs: `hsqs` at offset 0

## Cheat-Aware Design

Uses `guarded_ensure!` macro. See `.teams/KNOWLEDGE_anti-cheat-testing.md`.
