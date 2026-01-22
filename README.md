# recstrap

LevitateOS installer. Installs the system from the live ISO to disk, similar to `archinstall` for Arch Linux.

## Usage

```bash
recstrap /dev/vda
```

This will:
1. Partition the disk (GPT: 512MB EFI + rest as root)
2. Format partitions (FAT32 for EFI, ext4 for root)
3. Mount partitions
4. Extract squashfs to disk
5. Generate `/etc/fstab`
6. Install systemd-boot bootloader
7. Prompt for root password
8. Unmount and done

## Options

```
USAGE:
    recstrap [OPTIONS] <DISK>

ARGS:
    <DISK>    Target disk (e.g., /dev/vda, /dev/sda, /dev/nvme0n1)

OPTIONS:
    --efi-size <SIZE>    EFI partition size [default: 512M]
    --no-format          Skip partitioning and formatting (use existing partitions)
    --no-bootloader      Skip bootloader installation
    --squashfs <PATH>    Squashfs location [default: /media/cdrom/live/filesystem.squashfs]
    -h, --help           Print help
```

## Examples

```bash
# Standard installation
recstrap /dev/sda

# NVMe drive
recstrap /dev/nvme0n1

# Use existing partitions (manual partitioning)
recstrap --no-format /dev/vda

# Custom squashfs location
recstrap --squashfs /mnt/custom/filesystem.squashfs /dev/vda

# Larger EFI partition
recstrap --efi-size 1G /dev/vda
```

## Requirements

- Must be run from the LevitateOS live ISO
- Root privileges required
- Target disk will be **completely erased** (unless `--no-format`)

## Partition Layout

| Partition | Size | Type | Filesystem | Mount |
|-----------|------|------|------------|-------|
| 1 | 512MB | EFI System | FAT32 | /boot |
| 2 | Remaining | Linux root | ext4 | / |

## After Installation

1. Remove the installation media
2. Reboot
3. Log in as root with the password you set

## Building

```bash
cargo build --release
```

The release binary is optimized with LTO and symbol stripping for small size.

## License

MIT
