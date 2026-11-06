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

## What It Does NOT Do

- Run installation commands automatically
- Configure the system
- Install bootloader, set passwords, create users
- Any other installation step

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](../src/error.rs), and shared by recstrap, recchroot and recfstab.

## Protected Paths

The host system directories recchroot refuses are `PROTECTED_PATHS` in
`distro/spec/src/shared/paths.rs`.

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
