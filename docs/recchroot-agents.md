# CLAUDE.md - recchroot

## What is recchroot?

LevitateOS chroot helper. **Like arch-chroot, NOT like an installer.**

Sets up bind mounts, enters chroot, cleans up on exit. User runs commands inside.

## What Belongs Here

- Bind mount setup (/dev, /proc, /sys, /run)
- Chroot entry/exit
- Cleanup on error/signal

## What Does NOT Belong Here

| Don't put here | Put it in |
|----------------|-----------|
| System extraction | `tools/recstrap/` |
| Fstab generation | `tools/recfstab/` |
| Automatic installation | User does manually |

## Commands

```bash
cargo build --release
cargo clippy
```

## Usage

```bash
recchroot /mnt              # Interactive shell
recchroot /mnt passwd       # Run single command
recchroot /mnt bootctl install
```

## Mounts Created

1. `/proc` -> `<target>/proc`
2. `/sys` -> `<target>/sys`
3. `/dev` -> `<target>/dev`
4. `/run` -> `<target>/run`
5. `/sys/firmware/efi/efivars` -> `<target>/sys/firmware/efi/efivars` (if exists)
6. Copies `/etc/resolv.conf` for DNS

## Key Rule

Always cleanup mounts, even on error or signal.
