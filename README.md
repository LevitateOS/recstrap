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
genfstab -U /mnt >> /mnt/etc/fstab
arch-chroot /mnt                  # (or: chroot /mnt /bin/bash)
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
    --squashfs <PATH>    Squashfs location [default: /media/cdrom/live/filesystem.squashfs]
    -h, --help           Print help
```

## Examples

```bash
# Standard extraction to /mnt
recstrap /mnt

# Custom squashfs location
recstrap --squashfs /path/to/filesystem.squashfs /mnt
```

## What recstrap does

- Extracts the squashfs image to the target directory

## What recstrap does NOT do

- Partitioning (you run fdisk/parted)
- Formatting (you run mkfs)
- Mounting (you run mount)
- fstab generation (you run genfstab)
- Bootloader installation (you run bootctl)
- Password setting (you run passwd)
- User creation (you run useradd)

This is intentional. LevitateOS is for users who want control, like Arch.

## Requirements

- Must be run from the LevitateOS live ISO
- Root privileges required
- Target directory must be mounted and ready

## Building

```bash
cargo build --release
```

## License

MIT
