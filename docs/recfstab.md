# recfstab

Generates fstab entries from mounted filesystems. Like `genfstab` for Arch.

Outputs to stdout. You redirect it yourself.

## Status

**Beta.** Works for standard EFI + ext4 setups.

## Usage

```bash
# Mount partitions first
mount /dev/vda2 /mnt
mount /dev/vda1 /mnt/boot

# Generate fstab (append to file)
recfstab /mnt >> /mnt/etc/fstab

# Preview without writing
recfstab /mnt
```

## Options

```
recfstab [OPTIONS] <ROOT>

-L, --label      Use LABEL instead of UUID
-p, --partuuid   Use PARTUUID (GPT partition UUID)
-t, --partlabel  Use PARTLABEL
```

## Output Format

```
# /dev/vda2
UUID=a1b2c3d4-...    /         ext4    defaults    0    1

# /dev/vda1
UUID=ABCD-1234       /boot     vfat    defaults    0    2
```

## What It Does

1. Reads `/proc/mounts` for filesystems under `<ROOT>`
2. Reads `/proc/swaps` for swap (skips zram)
3. Looks up identifiers via `blkid`
4. Outputs fstab-formatted lines

## What It Does NOT Do

- Write to files (you redirect)
- Mount/unmount anything
- Validate the fstab syntax
- Any other installation step

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](../src/error.rs), and shared by recstrap, recchroot and recfstab.

## Requirements

- Root privileges
- util-linux (provides `findmnt`, `blkid`)
- Filesystems must be mounted

## Building

```bash
cargo build --release
```

## License

MIT

CLI integration checks use the real mount table and require Linux with a block-backed root filesystem plus `findmnt`/`blkid`. A command error or empty successful output fails the mount checks.

```bash
cargo test -p recfstab --test integration
```
