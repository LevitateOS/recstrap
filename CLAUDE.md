# CLAUDE.md - recstrap

## What is recstrap?

LevitateOS system extractor. **Like pacstrap, NOT like archinstall.**

Extracts EROFS rootfs from live ISO to target directory. That's it. User does everything else manually.

## What Belongs Here

- Rootfs extraction logic (EROFS)
- Pre-flight validation (root, paths, space, magic bytes)
- Error codes and cheat-aware checks

## What Does NOT Belong Here

| Don't put here | Put it in |
|----------------|-----------|
| Fstab generation | `tools/recfstab/` |
| Chroot setup | `tools/recchroot/` |
| Partitioning/formatting | User does manually |
| Bootloader installation | User does manually |

## Commands

```bash
cargo build --release    # LTO + strip enabled
cargo check # Unit tests are prohibited; use focused integration/runtime checks.
cargo clippy
```

## Usage

```bash
recstrap /mnt                    # Extract rootfs to /mnt (auto-detect .erofs path)
recstrap /mnt --rootfs /path     # Custom rootfs location (.erofs only)
recstrap /mnt --force            # Override non-empty/non-mount-point (not the host-root-fs guard)
recstrap /mnt --check            # Pre-flight validation only
```

## Error Codes

Every failure prints an `E0NN` code; the number is also the exit status. The codes are defined once, in [`src/error.rs`](src/error.rs), and shared by recstrap, recchroot and recfstab.

## Protected Paths (blocked even with --force)

`/`, `/bin`, `/boot`, `/boot/efi`, `/dev`, `/efi`, `/etc`, `/home`, `/lib`, `/lib32`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/usr/local`, `/var` (matched after resolving symlinks and `..`)

## Rootfs Format Detection

- `.erofs` extension → EROFS (mount + cp -aT)
- Anything else → invalid format (fails with E016)

Magic bytes are validated before extraction:
- EROFS: `0xe0f5e1e2` at offset 1024

## Installation Phases

1. **Environment Checks** - root, tools availability
2. **Target Directory Validation** - path, permissions, mount point, empty check
3. **Rootfs Validation** - format detection, magic bytes
4. **Format Validation & Tool Availability** - EROFS kernel support
5. **Pre-flight Check** - (optional with --check flag)
6. **Extraction** - EROFS mount+copy
7. **Post-Extraction Verification** - system is valid
8. **Security Hardening** - regenerate SSH host keys
9. **User Creation Setup** - (INTERACTIVE) optional user account creation

## User Creation Setup (Phase 9 - Interactive)

After extraction, if running interactively (stdin is a terminal and not --quiet), recstrap prompts:

```
Create initial user? [y/N]: y
Username: alice
```

Creates a setup script at `/root/setup-initial-user.sh` that user runs in chroot:

```bash
recchroot /mnt
bash /root/setup-initial-user.sh
```

The script:
- Creates user with home directory
- Prompts for the password with `passwd` inside the chroot (the password is never written to disk), then deletes itself
- Adds user to wheel for administration (sudo on Levitate, doas on Acorn; password authentication)

**Why this approach**:
- Preserves minimal pacstrap-like philosophy (extraction only)
- The username prompt happens before chroot; the password prompt happens inside it
- User can still set root password instead with: `passwd root`
- Script is optional - user can run manually or skip entirely

**Related**: See `distro-spec::shared::auth::README.md` for authentication architecture.

## Cheat-Aware Design

Uses `guarded_ensure!` macro. See `.teams/KNOWLEDGE_anti-cheat-testing.md`.
