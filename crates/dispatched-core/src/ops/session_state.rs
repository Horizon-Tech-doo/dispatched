//! Session state: read and set through a session receiver at credit 0 (no message is locked).

use crate::messaging::backend::{LockReceiver, MessagingBackend};
use crate::messaging::{ReceiveMode, ReceiverSpec, SessionChoice};
use crate::{Error, Result};
use std::time::Duration;

/// A session receiver at credit 0 on `session`: it locks the session, never a message.
async fn accept<B: MessagingBackend + ?Sized>(
    b: &mut B,
    address: &str,
    session: &str,
) -> Result<Box<dyn LockReceiver>> {
    let spec = ReceiverSpec {
        address: address.to_string(),
        mode: ReceiveMode::PeekLock,
        session: SessionChoice::Id(session.to_string()),
    };
    b.open_receiver(&spec, Duration::ZERO)
        .await?
        .ok_or_else(|| Error::Protocol(format!("session {session} in {address} was not accepted")))
}

/// The state of `session` on `address`.
pub(crate) async fn get<B: MessagingBackend + ?Sized>(
    b: &mut B,
    address: &str,
    session: &str,
) -> Result<Option<Vec<u8>>> {
    let mut rx = accept(b, address, session).await?;
    let state = rx.session_state().await;
    // Peek-lock: the close removes nothing.
    let _ = rx.close().await;
    state
}

/// Replace the state of `session` on `address`, or with `None` clear it.
pub(crate) async fn set<B: MessagingBackend + ?Sized>(
    b: &mut B,
    address: &str,
    session: &str,
    state: Option<Vec<u8>>,
) -> Result<()> {
    let mut rx = accept(b, address, session).await?;
    let done = rx.set_session_state(state.as_deref()).await;
    // Peek-lock: the close removes nothing.
    let _ = rx.close().await;
    done
}

#[cfg(test)]
mod tests {
    use crate::Error;
    use crate::messaging::fake::{FakeBroker, FakeMsg};
    use crate::ops::session_state;

    #[tokio::test]
    async fn session_state_round_trips_and_a_locked_session_is_a_conflict() {
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("x").in_session("A"));
        session_state::set(&mut b.clone(), "s", "A", Some(b"{\"step\":2}".to_vec()))
            .await
            .unwrap();
        assert_eq!(
            session_state::get(&mut b.clone(), "s", "A").await.unwrap(),
            Some(b"{\"step\":2}".to_vec())
        );
        session_state::set(&mut b.clone(), "s", "A", None)
            .await
            .unwrap();
        assert_eq!(
            session_state::get(&mut b.clone(), "s", "A").await.unwrap(),
            None
        );
        b.lock_session_elsewhere("s", "A");
        assert!(matches!(
            session_state::get(&mut b.clone(), "s", "A").await,
            Err(Error::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn session_state_locks_no_message_and_frees_the_session() {
        let b = FakeBroker::default();
        b.add("s", FakeMsg::active("x").in_session("A"));
        session_state::set(&mut b.clone(), "s", "A", Some(b"1".to_vec()))
            .await
            .unwrap();
        assert_eq!(b.held_locks(), 0);
        assert_eq!(b.all("s")[0].delivery_count, 0);
        assert_eq!(b.session_state("s", "A"), Some(b"1".to_vec()));
        // The receiver was closed: the session can be accepted again at once.
        session_state::set(&mut b.clone(), "s", "A", Some(b"2".to_vec()))
            .await
            .unwrap();
        assert_eq!(b.session_state("s", "A"), Some(b"2".to_vec()));
    }
}
