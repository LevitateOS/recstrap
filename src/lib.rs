//! Shared code for the install tools (`recstrap`, `recchroot`, `recfstab`, `recsetup`).

#[cfg(feature = "contracts")]
pub mod contract;
pub mod error;
pub mod fstab;
pub mod guard;
pub mod output;
pub mod session;

/// The tree on the live medium that only an A/B system gets: the A/B boot
/// step (`recsetup --boot ab`) copies it into the target before root A is
/// copied to root B; a mutable install never gets it. The builder fills it
/// with the recab desktop app on a desktop live medium.
pub const AB_ONLY_TREE: &str = "/usr/lib/recsetup/ab-only";
