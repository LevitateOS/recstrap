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
3. Copies `/etc/resolv.conf` for DNS
4. Runs `chroot <target> <command>`
5. Unmounts everything on exit

## What It Does NOT Do

- Run installation commands automatically
- Configure the system
- Install bootloader, set passwords, create users
- Any other installation step

## Exit Codes

| Code | Error |
|------|-------|
| 1 | Target does not exist |
| 2 | Not a directory |
| 3 | Can't create mount point |
| 4 | Mount failed |
| 5 | Unmount failed (warning) |
| 6 | Command failed |
| 7 | Not root |
| 8 | Protected path |

## Protected Paths

Cannot chroot into:

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

## Requirements

- Linux (uses Linux mount syscalls)
- Root privileges
- Valid Linux filesystem at target

## Building

```bash
cargo build --release
```

## License

MIT
