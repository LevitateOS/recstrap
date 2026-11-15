# recstrap

This crate builds the LevitateOS install tools: `recstrap` (below), [`recchroot`](docs/recchroot.md), [`recfstab`](docs/recfstab.md) and `recsetup`, which configures and boots the extracted system (time zone, locale, keymap, hostname, accounts, systemd-boot, boot entries, machine id, services) with plan/apply JSON and the target setup program installed by Recipe; its page is `docs/content/src/content/05-rec-tooling/14-recsetup.ts` in the superproject.

Extracts LevitateOS EROFS rootfs to a target directory. Like `pacstrap` for Arch - just extraction, nothing else.

## Status

**Beta.** Used by the install scenarios. Twelve pre-flight checks, distinct exit codes.

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

The shell setup flow needs no app: `recfstab /mnt` previews mount
entries, `recfstab /mnt --write` writes them, and `recsetup inspect --root /mnt`
describes the extracted system. List setup choices as text with
`recsetup inspect --root /mnt --list timezones` (also `locales`, `keymaps`,
`shells`, `groups`, `users`, or `services`). `recsetup plan` previews commands;
`recsetup apply` runs them and prints a line to stderr as each step starts
(`recsetup: step 3/12: ...`). Every tool's `--help` includes usage examples.

With `--json`, each stdout line is one event: `start` (the image's entry count and
size, once it is mounted and counted), `progress` (entries copied, counted from
`cp -v`, and bytes in use on the target's filesystem since the start; about five a
second), `verify`, and `done`. `--check --json` prints one `checked` event (the image,
its size, the target's free space). A failure prints the shared error document (`code`,
`message`, `remediation`) to stderr as its last JSON object; messages from `mount` or
`cp` can come before it. The types are `StrapEvent` and `ErrorPayload` in
[`src/output.rs`](src/output.rs), with JSON Schemas in `docs/schemas/` written by
`cargo xtask gen`.

The empty-target check (6) passes a target whose only contents are `lost+found`
and empty filesystems mounted inside it, so the layout disk-plan (recpart) mounts
at `/mnt`, `/mnt/boot` and `/mnt/var` counts as empty. A file anywhere on those
inner filesystems does not.

## What recstrap Does

1. Runs the pre-flight checks (below)
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
| 1 | Target is not a protected path (checked on the canonical path) | **Never** |
| 2 | Target is not on a host system filesystem | `--allow-host-root-fs` (never implied by `--force`) |
| 3 | Running as root | No |
| 4 | Target writable | No |
| 5 | Target is a mount point | `--allow-non-mountpoint` or `--force` |
| 6 | Target empty | `--allow-nonempty` or `--force` |
| 7 | Enough free space (at least 2 GiB) | No |
| 8 | Rootfs image found | No |
| 9 | Rootfs is a file | No |
| 10 | Rootfs is not inside the target | No |
| 11 | Rootfs is an EROFS image | No |
| 12 | The kernel supports EROFS | No |

(A missing or non-directory target is refused before these.) Check 2 refuses a target on the filesystem the host has mounted at `/` or at
any of the protected paths below that exist (`/home`, `/usr`, `/var`, `/boot`,
`/tmp`, ...). recstrap finds the mount that holds the target in
`/proc/self/mountinfo` and compares the filesystems' device numbers, so a
separate `/home` partition and a btrfs `/home` subvolume of the root filesystem
(Fedora) are both caught: `recstrap --force ~/x` fails with E026. It needs no
privilege and runs right after the protected-path check, before the root check.

## Protected Paths (Cannot Override)

The host system directories recstrap refuses as a target are `PROTECTED_PATHS`
in `target-root/src/lib.rs`, shared with recpart and recab.

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

The system recipes prepare the installed base system, including safe login
configuration, administrator rules, `/usr/libexec/recipe-system-setup` and the
root effects recipe Recipe runs after system changes. Networking, the desktop
and everything else above the base are group recipes installed through Recipe
(recsetup's software step on the release medium). Live autologin, live passwords, shell overrides
and test helpers are a separate overlay and are never deployed by recstrap.

`recsetup` validates user choices and invokes that target-side setup program for
timezone, locale, keymap, hostname, accounts, machine identity and services.

On the release medium, which carries the default software set
(`/usr/lib/recsetup/default-set`: its `list` and `recipes/`) and Recipe's package
cache (`/usr/share/recipe/cache`), every plan starts with the software step:
`recipe --system --root <target> --recipes-path /usr/lib/recsetup/default-set/recipes
install <the list>`, one system change that writes each group recipe into the
target with a receipt, offline from the cache, then runs the target's root effects
recipe once. It comes before the accounts (users join the groups the software
brings) and before the boot step (on A/B both roots get it); a medium without a
default set installs the base system alone. The desktop's keyboard layout is
looked up in the target when its step runs (`recsetup x11-keyboard`), after the
software step brought the keyboard data.
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
The initramfs it installs defaults, on both init systems, to the installed
system's own (`boot/initramfs-installed.img` on the live medium, built by the
release from the rootfs the ISO installs); the live medium's `boot/initramfs.img`
is refused (E030), since it only finds the live medium and ignores `root=`.
Passwords are supplied only at installation. This works offline; the
software step uses the Recipe and Ginux runtime the base system ships. Storage inspection,
ESP deployment and boot activation remain operations of the install tools and
recab. recstrap continues to validate and extract the selected image.

For A/B installation, recsetup finishes all requested personalization before
the boot step. It seeds machine identity and keys for enabled SSH services, then
puts the live medium's recab into root A with its `recab-started` service
(`recab install-unit`), and runs `recab init`, which copies root A to root B,
writes both boot entries and makes A the boot default. recab is on the live
medium only: a mutable install never gets it, and its one boot entry is written
by the live medium's `recab init --mode mutable`. Choose services and save
network configuration before this final boot step.

Apply records its request, target filesystem identity and completed commands in
`/var/lib/recsetup/apply.json`, under a separate apply lock. Passwords are never
stored. Retry the same unfinished request: completed steps are skipped and
completed password steps need no repeated stdin. An unfinished request cannot be
replaced with different settings. Completed pre-boot requests may be followed by
another configuration request. The boot command always runs again on retry; a
system that `recab init` already set up cannot be personalized afresh.
