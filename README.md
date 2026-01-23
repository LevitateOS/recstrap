# recstrap

Extracts LevitateOS squashfs to a target directory. Like `pacstrap` for Arch - just extraction, nothing else.

## Status

**Beta.** Used in E2E tests. 14 safety checks, distinct exit codes.

## Usage

```bash
# Standard use (from live ISO)
recstrap /mnt

# Custom squashfs location
recstrap --squashfs /path/to/filesystem.squashfs /mnt

# Pre-flight check only
recstrap --check /mnt

# Force (skip mount point + empty checks)
recstrap --force /mnt
```

## What recstrap Does

1. Validates target directory (14 checks)
2. Finds squashfs (auto-detect or `--squashfs`)
3. Runs `unsquashfs -f -d <target> <squashfs>`
4. Verifies extraction

## What recstrap Does NOT Do

- Partitioning → you run `fdisk`
- Formatting → you run `mkfs`
- Mounting → you run `mount`
- fstab → you run `recfstab`
- Bootloader → you run `bootctl`
- Users/passwords → you run `useradd`, `passwd`

This is intentional. Manual install like Arch.

## Safety Checks

| # | Check | Override |
|---|-------|----------|
| 1 | Root privileges | No |
| 2 | unsquashfs installed | No |
| 3 | Target exists | No |
| 4 | Target is directory | No |
| 5 | Path canonicalized | No |
| 6 | Not protected path | **Never** |
| 7 | Target writable | No |
| 8 | Is mount point | `--force` |
| 9 | Target empty | `--force` |
| 10 | Sufficient space (2GB) | No |
| 11 | Squashfs exists | No |
| 12 | Squashfs is file | No |
| 13 | Squashfs readable | No |
| 14 | Not recursive | No |

## Protected Paths (Cannot Override)

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Exit Codes

| Code | Error |
|------|-------|
| 1 | Target does not exist |
| 2 | Target not a directory |
| 3 | Target not writable |
| 4 | Squashfs not found |
| 5 | unsquashfs failed |
| 6 | Verification failed |
| 7 | unsquashfs not installed |
| 8 | Not root |
| 9 | Target not empty |
| 10 | Protected path |
| 11 | Not a mount point |
| 12 | Insufficient space |
| 13 | Squashfs not a file |
| 14 | Squashfs not readable |
| 15 | Squashfs inside target |

## Requirements

- Root privileges
- `unsquashfs` (squashfs-tools package)
- 2GB free space on target
- LevitateOS live ISO (or `--squashfs` flag)

## Building

```bash
cargo build --release
```

## License

MIT
