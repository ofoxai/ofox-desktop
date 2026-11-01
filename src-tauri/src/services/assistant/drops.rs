//! The files the user last dropped on the window. The window's drag-drop
//! event carries real paths; the renderer never sees them. It is told only a
//! single-use token and a count, and trades the token for an import. The
//! token belongs to the account and authority generation the files were
//! dropped under.
use super::host::IDENTITY_CHANGED;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a dropped set of files can still be imported.
pub const DROP_TTL: Duration = Duration::from_secs(60);

/// The token is unknown, was already used, or has expired: drop the files
/// again. One code for all three, so a guess learns nothing.
pub const DROP_UNAVAILABLE: &str = "assistant_drop_unavailable";

/// What is waiting to be imported: only the newest drop matters, so a new one
/// replaces it.
#[derive(Default)]
pub(super) struct DropSlot {
    pending: Option<Pending>,
}

struct Pending {
    token: String,
    owner: String,
    generation: u64,
    paths: Vec<PathBuf>,
    at: Instant,
}

impl DropSlot {
    /// Hold `paths`, dropped by `owner` under `generation`, in place of any
    /// earlier drop. Returns the token and how many paths it stands for.
    pub(super) fn put(
        &mut self,
        owner: &str,
        generation: u64,
        paths: Vec<PathBuf>,
        now: Instant,
    ) -> (String, usize) {
        let token = uuid::Uuid::new_v4().to_string();
        let count = paths.len();
        self.pending = Some(Pending {
            token: token.clone(),
            owner: owner.to_owned(),
            generation,
            paths,
            at: now,
        });
        (token, count)
    }

    /// The paths `token` stands for, once. A token this slot does not hold
    /// leaves what it holds alone; an expired drop, or one of another
    /// account or generation, is gone either way: nobody can use it.
    pub(super) fn take(
        &mut self,
        token: &str,
        owner: &str,
        generation: u64,
        now: Instant,
    ) -> Result<Vec<PathBuf>, String> {
        if self.pending.as_ref().map(|p| p.token.as_str()) != Some(token) {
            return Err(DROP_UNAVAILABLE.into());
        }
        let pending = self.pending.take().ok_or(DROP_UNAVAILABLE)?;
        if now.saturating_duration_since(pending.at) >= DROP_TTL {
            return Err(DROP_UNAVAILABLE.into());
        }
        if pending.owner != owner || pending.generation != generation {
            return Err(IDENTITY_CHANGED.into());
        }
        Ok(pending.paths)
    }

    /// Forget what is waiting (the authority it was bound to ended).
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "owner-a";

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_token_gives_back_what_was_dropped_once() {
        let mut slot = DropSlot::default();
        let start = Instant::now();
        let (token, count) = slot.put(OWNER, 3, paths(&["/d/a.txt", "/d/b.png"]), start);
        assert_eq!(count, 2);
        assert_eq!(
            slot.take(&token, OWNER, 3, start).unwrap(),
            paths(&["/d/a.txt", "/d/b.png"])
        );
        assert_eq!(
            slot.take(&token, OWNER, 3, start).unwrap_err(),
            DROP_UNAVAILABLE
        );
    }

    #[test]
    fn an_unknown_token_is_refused_and_keeps_the_drop_that_waits() {
        let mut slot = DropSlot::default();
        let start = Instant::now();
        assert_eq!(
            slot.take("never-issued", OWNER, 3, start).unwrap_err(),
            DROP_UNAVAILABLE
        );
        let (token, _) = slot.put(OWNER, 3, paths(&["/d/a.txt"]), start);
        for guess in ["", "/etc/passwd", "../x", &token.to_uppercase()] {
            assert_eq!(
                slot.take(guess, OWNER, 3, start).unwrap_err(),
                DROP_UNAVAILABLE,
                "{guess}"
            );
        }
        assert_eq!(slot.take(&token, OWNER, 3, start).unwrap().len(), 1);
    }

    #[test]
    fn a_drop_can_be_imported_for_under_60_seconds() {
        let mut slot = DropSlot::default();
        let start = Instant::now();
        let (late, _) = slot.put(OWNER, 3, paths(&["/d/a.txt"]), start);
        assert!(slot
            .take(&late, OWNER, 3, start + Duration::from_millis(59_999))
            .is_ok());

        let (expired, _) = slot.put(OWNER, 3, paths(&["/d/a.txt"]), start);
        assert_eq!(
            slot.take(&expired, OWNER, 3, start + DROP_TTL).unwrap_err(),
            DROP_UNAVAILABLE
        );
        // An expired drop is gone, not merely refused.
        assert_eq!(
            slot.take(&expired, OWNER, 3, start).unwrap_err(),
            DROP_UNAVAILABLE
        );
    }

    #[test]
    fn a_new_drop_replaces_the_one_that_waited() {
        let mut slot = DropSlot::default();
        let start = Instant::now();
        let (old, _) = slot.put(OWNER, 3, paths(&["/d/old.txt"]), start);
        let (new, _) = slot.put(OWNER, 3, paths(&["/d/new.txt"]), start);
        assert_ne!(old, new);
        assert_eq!(
            slot.take(&old, OWNER, 3, start).unwrap_err(),
            DROP_UNAVAILABLE
        );
        assert_eq!(
            slot.take(&new, OWNER, 3, start).unwrap(),
            paths(&["/d/new.txt"])
        );
    }

    #[test]
    fn a_token_is_refused_for_another_account_or_generation_and_then_gone() {
        let start = Instant::now();
        for (owner, generation) in [("owner-b", 3), (OWNER, 4)] {
            let mut slot = DropSlot::default();
            let (token, _) = slot.put(OWNER, 3, paths(&["/d/a.txt"]), start);
            assert_eq!(
                slot.take(&token, owner, generation, start).unwrap_err(),
                IDENTITY_CHANGED
            );
            assert_eq!(
                slot.take(&token, OWNER, 3, start).unwrap_err(),
                DROP_UNAVAILABLE
            );
        }
    }

    #[test]
    fn clearing_forgets_the_waiting_drop() {
        let mut slot = DropSlot::default();
        let start = Instant::now();
        let (token, _) = slot.put(OWNER, 3, paths(&["/d/a.txt"]), start);
        slot.clear();
        assert_eq!(
            slot.take(&token, OWNER, 3, start).unwrap_err(),
            DROP_UNAVAILABLE
        );
    }
}
