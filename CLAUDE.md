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
cargo test
cargo clippy
```

## Usage

```bash
recstrap /mnt                    # Extract rootfs to /mnt (auto-detect .erofs path)
recstrap /mnt --rootfs /path     # Custom rootfs location (.erofs only)
recstrap /mnt --force            # Override non-empty/non-mount-point
recstrap /mnt --check            # Pre-flight validation only
```

## Error Codes

| Code | Exit | Description |
|------|------|-------------|
| E001 | 1 | Target does not exist |
| E002 | 2 | Target is not a directory |
| E003 | 3 | Target not writable |
| E004 | 4 | Rootfs not found |
| E005 | 5 | Extraction failed |
| E006 | 6 | Verification failed |
| E007 | 7 | Required tool not installed |
| E008 | 8 | Must run as root |
| E009 | 9 | Target not empty |
| E010 | 10 | Protected system path |
| E011 | 11 | Not a mount point |
| E012 | 12 | Insufficient space |
| E013 | 13 | Rootfs is not a file |
| E014 | 14 | Rootfs not readable |
| E015 | 15 | Rootfs inside target |
| E016 | 16 | Invalid rootfs format (bad magic) |
| E017 | 17 | EROFS not supported by kernel |

## Protected Paths (blocked even with --force)

`/`, `/bin`, `/boot`, `/dev`, `/etc`, `/home`, `/lib`, `/lib64`, `/opt`, `/proc`, `/root`, `/run`, `/sbin`, `/srv`, `/sys`, `/tmp`, `/usr`, `/var`

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

After extraction, if running interactively (not --quiet or --force), recstrap prompts:

```
Create initial user? [y/N]: y
Username: alice
Password for alice: ****
```

Creates a setup script at `/root/setup-initial-user.sh` that user runs in chroot:

```bash
recchroot /mnt
bash /root/setup-initial-user.sh
```

The script:
- Creates user with home directory
- Sets password using chpasswd (securely, without shell expansion)
- Adds user to wheel group for passwordless sudo

**Why this approach**:
- Preserves minimal pacstrap-like philosophy (extraction only)
- Prompts happen BEFORE chroot, not inside
- User can still set root password instead with: `passwd root`
- Script is optional - user can run manually or skip entirely

**Related**: See `distro-spec::shared::auth::README.md` for authentication architecture.

## Cheat-Aware Design

Uses `guarded_ensure!` macro. See `.teams/KNOWLEDGE_anti-cheat-testing.md`.
