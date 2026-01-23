# recstrap

LevitateOS system extractor. Like `pacstrap` for Arch Linux - extracts the base system to a target directory.

**You do everything else manually.** Partitioning, formatting, mounting, fstab, bootloader, passwords - just like a real Arch install.

## Usage

```bash
# User does manual setup first (like Arch)
fdisk /dev/vda                    # Partition the disk
mkfs.fat -F32 /dev/vda1           # Format EFI partition
mkfs.ext4 /dev/vda2               # Format root partition
mount /dev/vda2 /mnt              # Mount root
mkdir -p /mnt/boot
mount /dev/vda1 /mnt/boot         # Mount boot

# Then extract the system
recstrap /mnt

# User does post-install manually
recfstab /mnt >> /mnt/etc/fstab
recchroot /mnt
passwd                            # Set root password
bootctl install                   # Install bootloader
# ... configure as needed
```

## Options

```
USAGE:
    recstrap [OPTIONS] <TARGET>

ARGS:
    <TARGET>    Target directory (e.g., /mnt)

OPTIONS:
    --squashfs <PATH>    Squashfs location (auto-detected if not specified)
    -f, --force          Force extraction even if target is not empty or not a mount point
    -q, --quiet          Quiet mode - minimal output for scripting
    -c, --check          Pre-flight check only - validate without extracting
    -h, --help           Print help
    -V, --version        Print version
```

## Examples

```bash
# Standard extraction to /mnt
recstrap /mnt

# Custom squashfs location
recstrap --squashfs /path/to/filesystem.squashfs /mnt

# Force overwrite existing files (also skips mount point check)
recstrap --force /mnt

# Scripting mode (no progress output)
recstrap --quiet /mnt

# Pre-flight check only (validate without extracting)
recstrap --check /mnt
```

## Safety Checks

recstrap validates before extraction:

1. **Root privileges** - Must run as root
2. **unsquashfs available** - Required tool must be installed
3. **Target exists** - Directory must exist
4. **Target is directory** - Not a file
5. **Path canonicalization** - Resolves symlinks and `..`
6. **Not protected path** - Blocks critical system dirs (CANNOT override)
7. **Target writable** - Write permission check
8. **Is mount point** - Prevents extracting to wrong filesystem (--force overrides)
9. **Target empty** - Prevents accidental overwrites (--force overrides). Note: `lost+found` directory (ext4) is ignored.
10. **Sufficient space** - At least 2GB required
11. **Squashfs exists** - File must exist
12. **Squashfs is file** - Not a directory
13. **Squashfs readable** - Can read the file
14. **Not recursive** - Squashfs not inside target

## Protected Paths

These paths are blocked (even with --force):

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

Use a proper mount point like `/mnt` or `/media/install`.

## What recstrap does NOT do

- Partitioning (you run fdisk/parted)
- Formatting (you run mkfs)
- Mounting (you run mount)
- fstab generation (you run recfstab)
- Bootloader installation (you run bootctl)
- Password setting (you run passwd)
- User creation (you run useradd)

This is intentional. LevitateOS is for users who want control, like Arch.

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
| E009 | 9 | Target directory not empty (use --force) |
| E010 | 10 | Target is a protected system path |
| E011 | 11 | Target is not a mount point (use --force) |
| E012 | 12 | Insufficient disk space |
| E013 | 13 | Squashfs is not a regular file |
| E014 | 14 | Squashfs is not readable |
| E015 | 15 | Squashfs is inside target directory |

## Requirements

- Must run as root
- unsquashfs must be installed (squashfs-tools)
- Target directory must be mounted and ready
- At least 2GB free space on target filesystem
- Running from LevitateOS live ISO (or specify --squashfs)

## Building

```bash
cargo build --release
```

## License

MIT
