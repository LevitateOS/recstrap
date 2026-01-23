# CLAUDE.md - Recstrap

## STOP. READ. THEN ACT.

Before modifying this crate, read `src/main.rs` to understand the extraction flow.

---

## What is recstrap?

LevitateOS system extractor. **Like pacstrap, NOT like archinstall.**

Extracts the squashfs from the live ISO to a target directory. That's it.
User does EVERYTHING else manually (partitioning, formatting, fstab, bootloader).

## Cheat-Aware Design

This tool uses `guarded_ensure!` macro based on Anthropic's [emergent misalignment research](https://www.anthropic.com/research/emergent-misalignment-reward-hacking).

Every validation check documents:
- **protects** - What user scenario this check protects
- **severity** - CRITICAL, HIGH, MEDIUM, or LOW
- **cheats** - Ways this check could be weakened to falsely pass
- **consequence** - What users experience if the check is cheated

When a check fails, full cheat documentation is printed. This creates friction against shortcuts.

## Development

```bash
cargo build --release    # LTO + strip enabled
cargo clippy
cargo test               # Unit + integration tests (51 total)
```

## Key Rules

1. **recstrap = pacstrap** - Just extract, nothing else
2. **Fail fast** - Check root, unsquashfs, target before doing work
3. **No automation** - User does manual install (like Arch)
4. **Distinct exit codes** - Each error has a unique exit code (1-15)
5. **Never extract to protected paths** - /, /usr, /etc, etc. CANNOT be overridden

## What recstrap does

```bash
recstrap /mnt                    # Extract squashfs to /mnt
recstrap /mnt --squashfs /path   # Custom squashfs location
recstrap /mnt --force            # Override non-empty/non-mount-point
recstrap /mnt --check            # Pre-flight validation only
recstrap /mnt --quiet            # Scripting mode
```

## What recstrap does NOT do

- Partitioning (user runs fdisk/parted)
- Formatting (user runs mkfs)
- Mounting (user runs mount)
- fstab generation (user runs recfstab)
- Bootloader (user runs bootctl)
- Password setting (user runs passwd)
- User creation (user runs useradd)

## Safety Checks (in order)

1. Root privileges (E008)
2. unsquashfs available (E007)
3. Target exists (E001)
4. Target is directory (E002)
5. Path canonicalization (resolve symlinks, ..)
6. Target is not protected path (E010) - **CANNOT be overridden**
7. Target writable (E003)
8. Target is mount point (E011) - skipped with --force
9. Target is empty (E009) - skipped with --force; `lost+found` ignored
10. Sufficient disk space (E012)
11. Squashfs exists (E004)
12. Squashfs is a file (E013)
13. Squashfs is readable (E014)
14. Squashfs not inside target (E015)

## Protected Paths

These are blocked even with --force:

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Error Codes

| Code | Exit | Description |
|------|------|-------------|
| E001 | 1 | Target directory does not exist |
| E002 | 2 | Target is not a directory |
| E003 | 3 | Target directory not writable |
| E004 | 4 | Squashfs image not found |
| E005 | 5 | unsquashfs command failed |
| E006 | 6 | Extracted system verification failed |
| E007 | 7 | unsquashfs not installed |
| E008 | 8 | Must run as root |
| E009 | 9 | Target directory not empty |
| E010 | 10 | Target is protected system path |
| E011 | 11 | Target is not a mount point |
| E012 | 12 | Insufficient disk space |
| E013 | 13 | Squashfs is not a regular file |
| E014 | 14 | Squashfs is not readable |
| E015 | 15 | Squashfs is inside target |

## Squashfs Search Paths

Auto-detected in order:
1. `/media/cdrom/live/filesystem.squashfs`
2. `/run/initramfs/live/filesystem.squashfs`
3. `/run/archiso/bootmnt/live/filesystem.squashfs`
4. `/mnt/cdrom/live/filesystem.squashfs`

Or specify with `--squashfs`.

## Testing

- Unit tests: 35 tests (error codes, helper functions, edge cases, lost+found handling)
- Integration tests: 20 tests (CLI, error paths, protected paths)
- Manual: actual extraction in QEMU with live ISO
