use crate::{MAX_RECORD_BYTES, VaultError};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub(super) instance: [u8; 32],
    pub(super) revision: u64,
    pub(super) hash: [u8; 32],
}

type Record = Option<Zeroizing<Vec<u8>>>;

pub trait Storage {
    fn load(&mut self) -> Result<Record, VaultError>;
    fn write(&mut self, bytes: Option<&[u8]>) -> Result<(), VaultError>;
}

pub struct State {
    pub(super) instance: [u8; 32],
    pub(super) revision: u64,
}

impl State {
    pub(super) fn load(&self, storage: &mut impl Storage) -> Result<(Stamp, Record), VaultError> {
        let bytes = storage.load()?;
        validate(bytes.as_deref().map(Vec::as_slice))?;
        Ok((self.stamp(bytes.as_deref().map(Vec::as_slice)), bytes))
    }

    pub(super) fn write(
        &mut self,
        storage: &mut impl Storage,
        expected: Stamp,
        bytes: Option<&[u8]>,
    ) -> Result<Stamp, VaultError> {
        validate(bytes)?;
        let (observed, _) = self.load(storage)?;
        if observed != expected {
            return Err(VaultError::Conflict);
        }
        // Rejected stale work must leave a newer workflow's cursor usable.
        // Once a native write is attempted, consume its revision even on failure.
        self.revision = self.revision.checked_add(1).ok_or(VaultError::Storage)?;
        storage.write(bytes)?;
        Ok(self.stamp(bytes))
    }

    fn stamp(&self, bytes: Option<&[u8]>) -> Stamp {
        let mut hash = Sha256::new();
        hash.update([u8::from(bytes.is_some())]);
        if let Some(value) = bytes {
            hash.update(value);
        }
        Stamp {
            instance: self.instance,
            revision: self.revision,
            hash: hash.finalize().into(),
        }
    }
}

fn validate(bytes: Option<&[u8]>) -> Result<(), VaultError> {
    if bytes.is_some_and(|value| value.len() > MAX_RECORD_BYTES) {
        return Err(VaultError::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_RECORD_BYTES;

    #[derive(Default)]
    struct Memory {
        bytes: Option<Zeroizing<Vec<u8>>>,
        fail_load: bool,
        fail_write: bool,
        fail_after_write: bool,
        writes: usize,
    }

    impl Storage for Memory {
        fn load(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
            if self.fail_load {
                return Err(VaultError::Storage);
            }
            Ok(self.bytes.clone())
        }

        fn write(&mut self, bytes: Option<&[u8]>) -> Result<(), VaultError> {
            self.writes += 1;
            if self.fail_write {
                return Err(VaultError::Storage);
            }
            self.bytes = bytes.map(|value| Zeroizing::new(value.to_vec()));
            if self.fail_after_write {
                return Err(VaultError::Storage);
            }
            Ok(())
        }
    }

    fn fixture() -> (State, Memory) {
        (
            State {
                instance: [1; 32],
                revision: 0,
            },
            Memory::default(),
        )
    }

    #[test]
    fn obsolete_queued_write_cannot_invalidate_a_newer_renewal_cursor() {
        let (mut state, mut memory) = fixture();
        let obsolete = state.load(&mut memory).unwrap().0;
        let newer = state
            .write(&mut memory, obsolete, Some(b"renewal-marker"))
            .unwrap();
        assert_eq!(
            state.write(&mut memory, obsolete, Some(b"obsolete-session")),
            Err(VaultError::Conflict)
        );
        state
            .write(&mut memory, newer, Some(b"renewed-session"))
            .unwrap();
        assert_eq!(
            memory.bytes.as_deref().unwrap().as_slice(),
            b"renewed-session"
        );
    }

    #[test]
    fn save_clear_and_exact_size_boundary() {
        let (mut state, mut memory) = fixture();
        let (initial, bytes) = state.load(&mut memory).unwrap();
        assert!(bytes.is_none());
        let data = vec![b'x'; MAX_RECORD_BYTES];
        let saved = state.write(&mut memory, initial, Some(&data)).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(state.load(&mut memory).unwrap().1.unwrap().as_slice(), data);
        let cleared = state.write(&mut memory, saved, None).unwrap();
        assert_eq!(cleared.revision, 2);
        assert!(state.load(&mut memory).unwrap().1.is_none());
        let absent = state.write(&mut memory, cleared, None).unwrap();
        assert_eq!(absent.revision, 3);
        assert_eq!(
            state.write(&mut memory, absent, Some(&vec![0; MAX_RECORD_BYTES + 1])),
            Err(VaultError::InvalidRequest)
        );
        assert_eq!(memory.writes, 3);
    }

    #[test]
    fn absent_and_identical_record_aba_cannot_restore_old_writes() {
        let (mut state, mut memory) = fixture();
        let old = state.load(&mut memory).unwrap().0;
        let first = state.write(&mut memory, old, Some(b"new-session")).unwrap();
        let second = state
            .write(&mut memory, first, Some(b"new-session"))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            state.write(&mut memory, first, None),
            Err(VaultError::Conflict)
        );
        let current = state.load(&mut memory).unwrap().0;
        state.write(&mut memory, current, None).unwrap();
        assert_eq!(
            state.write(&mut memory, old, Some(b"obsolete")),
            Err(VaultError::Conflict)
        );
        assert!(memory.bytes.is_none());
    }

    #[test]
    fn restart_and_external_change_reject_stale_generation() {
        let (mut state, mut memory) = fixture();
        let old = state.load(&mut memory).unwrap().0;
        state.instance = [2; 32];
        assert_eq!(
            state.write(&mut memory, old, None),
            Err(VaultError::Conflict)
        );
        let loaded = state.load(&mut memory).unwrap().0;
        memory.bytes = Some(Zeroizing::new(b"external".to_vec()));
        assert_eq!(
            state.write(&mut memory, loaded, None),
            Err(VaultError::Conflict)
        );
        assert_eq!(memory.bytes.as_deref().unwrap().as_slice(), b"external");
        assert_eq!(memory.writes, 0);
    }

    #[test]
    fn failures_advance_revision_and_never_repeat_native_write() {
        let (mut state, mut memory) = fixture();
        let old = state.load(&mut memory).unwrap().0;
        memory.fail_write = true;
        assert_eq!(
            state.write(&mut memory, old, Some(b"secret-canary")),
            Err(VaultError::Storage)
        );
        assert_eq!(state.revision, 1);
        memory.fail_write = false;
        assert_eq!(
            state.write(&mut memory, old, Some(b"secret-canary")),
            Err(VaultError::Conflict)
        );
        assert_eq!(memory.writes, 1);
        let loaded = state.load(&mut memory).unwrap().0;
        memory.fail_load = true;
        assert_eq!(
            state.write(&mut memory, loaded, None),
            Err(VaultError::Storage)
        );
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn oversized_native_record_and_revision_overflow_fail_closed() {
        let (mut state, mut memory) = fixture();
        let old = state.load(&mut memory).unwrap().0;
        memory.bytes = Some(Zeroizing::new(vec![0; MAX_RECORD_BYTES + 1]));
        assert_eq!(state.load(&mut memory), Err(VaultError::InvalidRequest));
        memory.bytes = None;
        state.revision = u64::MAX;
        let current = state.load(&mut memory).unwrap().0;
        assert_ne!(old, current);
        assert_eq!(
            state.write(&mut memory, current, None),
            Err(VaultError::Storage)
        );
        assert_eq!(memory.writes, 0);
    }

    #[test]
    fn unknown_native_write_result_preserves_new_generation_and_actual_bytes() {
        let (mut state, mut memory) = fixture();
        let old = state.load(&mut memory).unwrap().0;
        memory.fail_after_write = true;
        assert_eq!(
            state.write(&mut memory, old, Some(b"committed-before-error")),
            Err(VaultError::Storage)
        );
        memory.fail_after_write = false;
        assert_eq!(
            state.write(&mut memory, old, None),
            Err(VaultError::Conflict)
        );
        let (fresh, bytes) = state.load(&mut memory).unwrap();
        assert_eq!(fresh.revision, 1);
        assert_eq!(bytes.unwrap().as_slice(), b"committed-before-error");
        state.write(&mut memory, fresh, None).unwrap();
    }
}
