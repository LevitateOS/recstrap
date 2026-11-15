# recfstab

Generates fstab entries from mounted filesystems. Like `genfstab` for Arch.

Outputs to stdout. You redirect it yourself, or let `--write` append to
`<ROOT>/etc/fstab`. Partitions are named by PARTUUID unless you ask otherwise:
the boot entries name the roots by PARTUUID too, and it survives `recab copy`
creating the other root's filesystem anew.

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

# Append to /mnt/etc/fstab; refused (E036) when it already describes those mount points
recfstab --write /mnt

# ... replacing the lines (and recfstab's comment line above each) that describe them
recfstab --write --replace /mnt

# The entries, the fstab's current lines and warnings as JSON (the base-system app)
recfstab --json /mnt
```

`--json` prints a `FstabResult` ([`src/output.rs`](../src/output.rs)): the entries
(each with the text it writes), the active lines already in `<ROOT>/etc/fstab` marked
`conflicts` when they describe a mount point (or swap device) an entry describes, and
warnings: nothing mounted at the root itself, no vfat EFI system partition at
`/boot`, `/efi` or `/boot/efi`, or no `<ROOT>/etc` yet. Warnings never stop a
write. Errors go to stderr as the shared JSON error document.

## Options

```
recfstab [OPTIONS] <ROOT>

-p, --partuuid   Use PARTUUID (GPT partition UUID); the default
-U, --uuid       Use the filesystem UUID
-L, --label      Use the filesystem LABEL
-t, --partlabel  Use PARTLABEL
-w, --write      Append the entries to <ROOT>/etc/fstab
    --replace    With --write: replace the fstab's entries for those mount points
    --json       Print the entries, current lines and warnings as JSON
```

## Output Format

```
# /dev/vda2
PARTUUID=a1b2c3d4-...    /         ext4    defaults    0    1

# /dev/vda1
PARTUUID=ab12cd34-...    /boot     vfat    defaults    0    2
```

## What It Does

1. Reads the mount table (`/proc/self/mountinfo`) for filesystems under `<ROOT>`
2. Reads active swap from `swapon`
3. Looks up identifiers (`blkid` tags)
4. Outputs fstab-formatted lines

## What It Does NOT Do

- Write to files, unless asked (`--write`)
- Mount/unmount anything
- Validate the fstab syntax
- Any other installation step

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](../src/error.rs), and shared by recstrap, recchroot, recfstab and recsetup.

## Requirements

- Root privileges
- util-linux (provides `swapon`, `lsblk`, `blkid`); mounts are read from `/proc/self/mountinfo`, swap from `swapon`
- Filesystems must be mounted

## Building

```bash
cargo build --release
```

## License

MIT OR Apache-2.0

CLI integration checks use the real mount table and require Linux with a block-backed root filesystem plus `blkid`. A command error or empty successful output fails the mount checks.

```bash
cargo test -p recfstab --test integration
```
