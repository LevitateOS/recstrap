# recstrap

This crate builds the LevitateOS install tools: `recstrap` (below), [`recchroot`](docs/recchroot.md), [`recfstab`](docs/recfstab.md) and `recsetup`, which configures and boots the extracted system (time zone, locale, keymap, hostname, accounts, systemd-boot, boot entries, machine id, services) with plan/apply JSON and the target setup program installed by Recipe; its page is `docs/content/src/content/05-rec-tooling/14-recsetup.ts` in the superproject.

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

# Skip the mount point and empty checks (same as --force)
recstrap --allow-non-mountpoint --allow-nonempty /mnt
```

## What recstrap Does

1. Validates target directory (15 checks)
2. Finds rootfs (auto-detect or `--rootfs`)
3. Mounts EROFS read-only and copies files into target
4. Verifies extraction

## What recstrap Does NOT Do

- Partitioning → you run `fdisk`
- Formatting → you run `mkfs`
- Mounting → you run `mount`
- fstab → you run `recfstab`
- Configuration, users, bootloader and services → you run `recsetup` (or the commands its plan prints)

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
| 8 | Not on the host's root filesystem | `--allow-host-root-fs` (never implied by `--force`) |
| 9 | Is mount point | `--allow-non-mountpoint` or `--force` |
| 10 | Target empty | `--allow-nonempty` or `--force` |
| 11 | Sufficient space (2GB) | No |
| 12 | Rootfs exists | No |
| 13 | Rootfs is file | No |
| 14 | Rootfs readable | No |
| 15 | Not recursive | No |

## Protected Paths (Cannot Override)

The host system directories recstrap refuses as a target are `PROTECTED_PATHS`
in `src/guard.rs`.

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](src/error.rs), and shared by recstrap, recchroot, recfstab and recsetup.

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

MIT OR Apache-2.0

## Recipe and deployment

The system recipes prepare a complete installed filesystem, including safe login
configuration, default networking, administrator rules and
`/usr/libexec/recipe-system-setup`. Live autologin, live passwords, shell overrides
and test helpers are a separate overlay and are never deployed by recstrap.

`recsetup` validates user choices and invokes that target-side setup program for
timezone, locale, keymap, hostname, accounts, machine identity and services.
Passwords are supplied only at installation. This works offline without shipping
a compiler image or Ginux build runtime on the live ISO. Storage inspection,
ESP deployment and boot activation remain operations of the install tools and
recab. recstrap continues to validate and extract the selected image.
