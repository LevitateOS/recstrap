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
/// with the recab desktop app on the release medium.
pub const AB_ONLY_TREE: &str = "/usr/lib/recsetup/ab-only";

/// The tree on the live medium that only a mutable system gets, the mirror
/// image of [`AB_ONLY_TREE`]: the mutable boot step (`recsetup --boot
/// mutable`) copies it into the target; an A/B install never gets it. The
/// builder fills it with the Updater desktop app on the release medium.
pub const MUTABLE_ONLY_TREE: &str = "/usr/lib/recsetup/mutable-only";

/// The default software set on the release medium: `list` (the group recipes
/// of the variant's `[desktop].recipes`, one name per line, in install order)
/// and `recipes/` (those recipes and their deps). recsetup's software step
/// installs the list into the target through Recipe with `recipes/` as its
/// recipes folder; a medium without it (a test ISO) installs the base alone.
/// It is on the live medium's overlay only, never in an installed system.
pub const DEFAULT_SET_DIR: &str = "/usr/lib/recsetup/default-set";

/// Recipe's offline package cache (`sha256/<hex>`), which Recipe reads before
/// the network whenever this directory exists. The release medium no longer
/// carries the files the default set pins: `recupdate fetch-parts` unpacks the
/// release's category parts into this folder (its default is a literal copy of
/// this constant, `DEFAULT_RECIPE_CACHE_DIR` in tools/recupdate/src/parts.rs;
/// keep them equal) and recsetup's software step does so on the target disk.
/// An installed system never has it.
pub const RECIPE_CACHE_DIR: &str = "/usr/share/recipe/cache";

/// Where recsetup's software step keeps the fetched package parts, relative to the
/// target root: the parts' download folder and the unpacked package cache, on the target
/// disk. The step removes the folder when it ends; the install scenario checks that.
pub const FETCHED_PARTS_DIR: &str = "/var/tmp/recipe-parts";
