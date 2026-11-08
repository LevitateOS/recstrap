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

# Progress as JSON lines on stdout, errors as JSON on stderr (the base-system app)
recstrap --json /mnt
recstrap --check --json /mnt
```

With `--json`, each stdout line is one event: `start` (the image's entry count and
size, once it is mounted and counted), `progress` (entries copied, counted from
`cp -v`, and bytes in use on the target's filesystem since the start; about five a
second), `verify`, and `done`. `--check --json` prints one `checked` event (the image,
its size, the target's free space). A failure prints the shared error document (`code`,
`message`, `remediation`) to stderr as its last JSON object; messages from `mount` or
`cp` can come before it. The types are `StrapEvent` and `ErrorPayload` in
[`src/output.rs`](src/output.rs), with JSON Schemas in `docs/schemas/` written by
`cargo xtask gen`.

The empty-target check (10) passes a target whose only contents are `lost+found`
and empty filesystems mounted inside it, so the layout disk-plan (recpart) mounts
at `/mnt`, `/mnt/boot` and `/mnt/var` counts as empty. A file anywhere on those
inner filesystems does not.

## What recstrap Does

1. Validates target directory (15 checks)
2. Finds rootfs (auto-detect or `--rootfs`)
3. Mounts EROFS read-only and copies files into target
4. Verifies extraction

The image is mounted in a private mount namespace that recstrap enters before
mounting (as recchroot does): the mount is never visible to the host and
disappears with the process, so even a killed recstrap (SIGKILL, a dropped SSH
session) leaves no loop mount behind. A hangup (SIGHUP), like an interrupt,
stops the copy and recstrap then unmounts and removes its directory itself.
Not root, recstrap runs in the host's namespace and says so; as root, a
namespace that cannot be created stops it before it mounts anything. The
mount directory is made under `/run`, not `$TMPDIR`; an image file is mounted
through a loop device, a partition holding the image directly.

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
| 8 | Not on a host system filesystem | `--allow-host-root-fs` (never implied by `--force`) |
| 9 | Is mount point | `--allow-non-mountpoint` or `--force` |
| 10 | Target empty | `--allow-nonempty` or `--force` |
| 11 | Sufficient space (2GB) | No |
| 12 | Rootfs exists | No |
| 13 | Rootfs is file | No |
| 14 | Rootfs readable | No |
| 15 | Not recursive | No |

Check 8 refuses a target on the filesystem the host has mounted at `/` or at
any of the protected paths below that exist (`/home`, `/usr`, `/var`, `/boot`,
`/tmp`, ...). recstrap finds the mount that holds the target in
`/proc/self/mountinfo` and compares the filesystems' device numbers, so a
separate `/home` partition and a btrfs `/home` subvolume of the root filesystem
(Fedora) are both caught: `recstrap --force ~/x` fails with E026. It needs no
privilege and runs right after the protected-path check, before the root check.

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
Both read what the system is from the distro profile in its `/etc/os-release`
(the `REC_*` fields the builder writes: package backend, init system, libc,
privilege tool, desktop, kernel line), never by probing for programs; a target
without them is refused (E029). recstrap's own check after extraction is that
the extracted tree carries that profile (E006 otherwise).
Each step that runs in the target gets its own chroot session (the binds and
DNS recchroot sets up, in recsetup's own process), so no mount outlives the step.

For `--boot`, `recsetup plan` (and so `apply`) requires the installed system's
root partition to be mounted exactly at `--root` (E011 otherwise) and checks the
boot devices against it, comparing device numbers so `/dev/disk/by-*` names and
other symlinks work: `--root-device` (mutable) or `--slot-a` (A/B) must be the
partition mounted at `--root`, and `--slot-b` must not be (E035). The ESP may
be on another disk.
The initramfs it installs defaults, on systemd systems, to the installed
system's own (`boot/initramfs-installed.img` on the live medium, built by the
release from the rootfs the ISO installs); the live medium's `boot/initramfs.img`
is refused (E030), since it only finds the live medium and ignores `root=`.
Passwords are supplied only at installation. This works offline without shipping
a compiler image or Ginux build runtime on the live ISO. Storage inspection,
ESP deployment and boot activation remain operations of the install tools and
recab. recstrap continues to validate and extract the selected image.
