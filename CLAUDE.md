# CLAUDE.md - recstrap

## What is recstrap?

LevitateOS system extractor. **Like pacstrap, NOT like archinstall.**

Extracts squashfs from live ISO to target directory. That's it. User does everything else manually.

## What Belongs Here

- Squashfs extraction logic
- Pre-flight validation (root, paths, space)
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
recstrap /mnt                    # Extract squashfs to /mnt
recstrap /mnt --squashfs /path   # Custom squashfs location
recstrap /mnt --force            # Override non-empty/non-mount-point
recstrap /mnt --check            # Pre-flight validation only
```

## Error Codes

| Code | Exit | Description |
|------|------|-------------|
| E001 | 1 | Target does not exist |
| E002 | 2 | Target is not a directory |
| E003 | 3 | Target not writable |
| E004 | 4 | Squashfs not found |
| E005 | 5 | unsquashfs failed |
| E006 | 6 | Verification failed |
| E007 | 7 | unsquashfs not installed |
| E008 | 8 | Must run as root |
| E009 | 9 | Target not empty |
| E010 | 10 | Protected system path |
| E011 | 11 | Not a mount point |
| E012 | 12 | Insufficient space |

## Protected Paths (blocked even with --force)

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Cheat-Aware Design

Uses `guarded_ensure!` macro. See `.teams/KNOWLEDGE_anti-cheat-testing.md`.
