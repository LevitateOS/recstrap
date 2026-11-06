# CLAUDE.md - recfstab

## What is recfstab?

LevitateOS fstab generator. **Like genfstab, NOT like an installer.**

Reads mounted filesystems under a root directory and outputs fstab entries. User redirects to file.

## What Belongs Here

- Fstab entry generation
- UUID/LABEL detection
- Mount point discovery

## What Does NOT Belong Here

| Don't put here | Put it in |
|----------------|-----------|
| System extraction | `tools/recstrap/` |
| Chroot setup | `tools/recchroot/` |
| Writing files directly | User redirects output |

## Commands

```bash
cargo build --release
cargo clippy
```

## Usage

```bash
recfstab /mnt >> /mnt/etc/fstab    # Generate with UUIDs
recfstab -L /mnt >> /mnt/etc/fstab # Generate with LABELs
```

## Key Rule

Output to stdout only. User handles file redirection.
