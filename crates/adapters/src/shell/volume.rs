//! The volume behind the [`Volume`] port: `statvfs(3)`, through `rustix`'s
//! safe wrapper, so no `unsafe` is needed to ask it.

use std::path::Path;

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::volume::Volume;

/// The real volume, asked with `statvfs`.
pub struct Statvfs;

impl Volume for Statvfs {
    fn free_bytes(&self, path: &Path) -> Outcome<u64> {
        let stat = rustix::fs::statvfs(path).map_err(|e| {
            Halt::Failed(format!(
                "the free space of {} cannot be read: {e}",
                path.display()
            ))
        })?;
        // What an unprivileged process may write (`f_bavail`), in fragments.
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_of_a_real_folder_has_room_and_a_missing_path_is_an_error() {
        let free = Statvfs
            .free_bytes(&std::env::temp_dir())
            .expect("the temp folder's volume answers");
        assert!(free > 0, "{free}");
        let missing = Statvfs.free_bytes(Path::new("/no/such/harness/folder"));
        assert!(
            matches!(&missing, Err(Halt::Failed(why)) if why.contains("/no/such/harness/folder")),
            "{missing:?}"
        );
    }
}
