# recchroot

Enters chroot with proper bind mounts. Like `arch-chroot` for Arch.

Sets up mounts, runs your command, cleans up on exit.

## Status

**Beta.** Works for standard installations.

## Usage

```bash
# After recstrap
recstrap /mnt

# Enter chroot (interactive shell)
recchroot /mnt

# Run single command
recchroot /mnt passwd

# Run bootloader install
recchroot /mnt bootctl install
```

## What It Does

1. Bind mounts `/proc`, `/sys`, `/dev`, `/run`
2. Bind mounts `/sys/firmware/efi/efivars` (if UEFI)
3. Bind-mounts the host `/etc/resolv.conf` over the target's for DNS
4. Runs `chroot <target> <command>`
5. Unmounts everything on exit

All of this happens in a private mount namespace that recchroot enters before
its first mount. The binds are never visible to the host, nothing propagates
in either direction, and the kernel removes them when recchroot and the
command are gone, however they end: a SIGKILL, a dropped SSH session or a
closed terminal cannot leave the host's `/dev`, `/sys`, `/proc` or `/run` bound
under the target. On SIGHUP, SIGINT, SIGTERM or SIGQUIT recchroot itself keeps
running, lets the command stop, and unmounts. Mounts the host already has (the
target at `/mnt`, its `/boot`) are visible inside, and writes to them reach the
disk.

A consequence: a mount made by the command inside the chroot (say
`mount /dev/vda1 /boot`) also ends with the session. Mount the target's
filesystems on the host, before running recchroot. Without the privilege to
create the namespace, recchroot runs in the host's namespace as before.

## What It Does NOT Do

- Run installation commands automatically
- Configure the system
- Install bootloader, set passwords, create users
- Any other installation step

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](../src/error.rs), and shared by recstrap, recchroot, recfstab and recsetup.

## Protected Paths

The host system directories recchroot refuses are `PROTECTED_PATHS` in
`src/guard.rs`.

## Requirements

- Linux (uses Linux mount syscalls)
- Root privileges
- Valid Linux filesystem at target

## Building

```bash
cargo build --release
```

## License

MIT OR Apache-2.0
