# recstrap

Extracts LevitateOS EROFS rootfs to a target directory. Like `pacstrap` for Arch - just extraction, nothing else.

## Status

**Beta.** Used in E2E tests. 14 safety checks, distinct exit codes.

## Usage

```bash
# Standard use (from live ISO)
recstrap /mnt

# Custom EROFS location
recstrap --rootfs /path/to/filesystem.erofs /mnt

# Pre-flight check only
recstrap --check /mnt

# Force (skip mount point + empty checks)
recstrap --force /mnt
```

## What recstrap Does

1. Validates target directory (14 checks)
2. Finds rootfs (auto-detect or `--rootfs`)
3. Mounts EROFS read-only and copies files into target
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
| 2 | EROFS kernel support available | No |
| 3 | Target exists | No |
| 4 | Target is directory | No |
| 5 | Path canonicalized | No |
| 6 | Not protected path | **Never** |
| 7 | Target writable | No |
| 8 | Is mount point | `--force` |
| 9 | Target empty | `--force` |
| 10 | Sufficient space (2GB) | No |
| 11 | Rootfs exists | No |
| 12 | Rootfs is file | No |
| 13 | Rootfs readable | No |
| 14 | Not recursive | No |

## Protected Paths (Cannot Override)

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Exit Codes

| Code | Error |
|------|-------|
| 1 | Target does not exist |
| 2 | Target not a directory |
| 3 | Target not writable |
| 4 | Rootfs not found |
| 5 | Extraction failed |
| 6 | Verification failed |
| 7 | Required tool missing |
| 8 | Not root |
| 9 | Target not empty |
| 10 | Protected path |
| 11 | Not a mount point |
| 12 | Insufficient space |
| 13 | Rootfs not a file |
| 14 | Rootfs not readable |
| 15 | Rootfs inside target |
| 16 | Invalid rootfs format |
| 17 | EROFS not supported by kernel |

## Requirements

- Root privileges
- EROFS support in the running kernel (`erofs` in `/proc/filesystems`)
- 2GB free space on target
- LevitateOS live ISO (or `--rootfs /path/to/filesystem.erofs`)

## Building

```bash
cargo build --release
```

## License

MIT
