#[cfg(any(target_os = "macos", test))]
use crate::protocol;
use crate::{
    MAX_RECORD_BYTES, VaultError,
    protocol::{Request, Response},
    state::Stamp,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
enum Cursor {
    Initial,
    Loaded(Stamp),
    Invalidated,
}

enum Channel {
    #[cfg(target_os = "macos")]
    Native(rama_net_apple_xpc::XpcConnection),
    #[cfg(test)]
    Test(tokio::sync::mpsc::Sender<TestCall>),
}

#[cfg(test)]
type TestCall = (
    Zeroizing<Vec<u8>>,
    tokio::sync::oneshot::Sender<Result<Zeroizing<Vec<u8>>, VaultError>>,
);

/// One serialized workflow's connection to the fixed local vault.
///
/// Keep the caller's cross-process workflow lock for this client's entire lifetime.
/// No request is automatically replayed. Drop the client after any failure.
pub struct Client {
    channel: Channel,
    cursor: Cursor,
}

impl Client {
    /// Connect using the compiled certificate pin and fixed vault identity.
    ///
    /// The native connection is lazy; authentication failures can arrive on load.
    /// # Errors
    /// Returns a sanitized error if the platform, pin, or connection is unavailable.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(
            clippy::missing_const_for_fn,
            reason = "The native implementation cannot be const."
        )
    )]
    pub fn connect() -> Result<Self, VaultError> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                channel: Channel::Native(crate::native::connect()?),
                cursor: Cursor::Initial,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(VaultError::Unavailable)
        }
    }

    /// Load the record and retain its compare-and-swap generation.
    ///
    /// # Errors
    /// Returns a sanitized error and prevents further use after failure or cancellation.
    pub async fn load(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
        let response = self.exchange(Request::Load).await?;
        self.cursor = Cursor::Loaded(response.stamp);
        Ok(response.bytes)
    }

    /// Save once against the last loaded generation; initially loads if necessary.
    ///
    /// # Errors
    /// Returns a sanitized error; an unknown write must never be retried.
    pub async fn save(&mut self, bytes: &[u8]) -> Result<(), VaultError> {
        if bytes.len() > MAX_RECORD_BYTES {
            self.cursor = Cursor::Invalidated;
            return Err(VaultError::InvalidRequest);
        }
        self.write(Some(bytes)).await
    }

    /// Delete once against the last loaded generation; initially loads if necessary.
    ///
    /// # Errors
    /// Returns a sanitized error; an unknown deletion must never be retried.
    pub async fn clear(&mut self) -> Result<(), VaultError> {
        self.write(None).await
    }

    async fn write(&mut self, bytes: Option<&[u8]>) -> Result<(), VaultError> {
        if matches!(self.cursor, Cursor::Initial) {
            self.load().await?;
        }
        let Cursor::Loaded(stamp) = self.cursor else {
            return Err(VaultError::Invalidated);
        };
        let response = self.exchange(Request::Write(stamp, bytes)).await?;
        if response.bytes.is_some()
            || response.stamp.instance != stamp.instance
            || response.stamp.revision <= stamp.revision
        {
            return Err(VaultError::InvalidRequest);
        }
        self.cursor = Cursor::Loaded(response.stamp);
        Ok(())
    }

    #[cfg(any(target_os = "macos", test))]
    async fn exchange(&mut self, request: Request<'_>) -> Result<Response, VaultError> {
        if matches!(self.cursor, Cursor::Invalidated) {
            return Err(VaultError::Invalidated);
        }
        // This assignment precedes the first await, including a send that outlives
        // its caller. Only a complete, valid response restores the cursor.
        self.cursor = Cursor::Invalidated;
        let frame = protocol::encode_request(request);
        let response = match &self.channel {
            #[cfg(target_os = "macos")]
            Channel::Native(connection) => crate::native::send(connection, frame).await?,
            #[cfg(test)]
            Channel::Test(sender) => {
                let (reply, receive) = tokio::sync::oneshot::channel();
                sender
                    .send((frame, reply))
                    .await
                    .map_err(|_| VaultError::Transport)?;
                receive.await.map_err(|_| VaultError::Transport)??
            }
        };
        protocol::decode_response(&response)
    }

    #[cfg(all(not(target_os = "macos"), not(test)))]
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "The platform-independent API remains async."
    )]
    async fn exchange(&mut self, _: Request<'_>) -> Result<Response, VaultError> {
        self.cursor = Cursor::Invalidated;
        Err(VaultError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Client, tokio::sync::mpsc::Receiver<TestCall>) {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        (
            Client {
                channel: Channel::Test(sender),
                cursor: Cursor::Initial,
            },
            receiver,
        )
    }

    fn response(revision: u64) -> Zeroizing<Vec<u8>> {
        protocol::encode_response(Ok(Response {
            stamp: Stamp {
                instance: [1; 32],
                revision,
                hash: [2; 32],
            },
            bytes: None,
        }))
    }

    #[tokio::test]
    async fn initial_save_loads_once_and_success_preserves_new_cursor() {
        let (mut client, mut receive) = fixture();
        let peer = tokio::spawn(async move {
            let (request, reply) = receive.recv().await.unwrap();
            assert!(matches!(
                protocol::decode_request(&request),
                Ok(Request::Load)
            ));
            reply.send(Ok(response(0))).unwrap();
            let (request, reply) = receive.recv().await.unwrap();
            assert!(matches!(
                protocol::decode_request(&request),
                Ok(Request::Write(
                    Stamp { revision: 0, .. },
                    Some(b"synthetic")
                ))
            ));
            reply.send(Ok(response(1))).unwrap();
            let (request, reply) = receive.recv().await.unwrap();
            assert!(matches!(
                protocol::decode_request(&request),
                Ok(Request::Write(Stamp { revision: 1, .. }, None))
            ));
            reply.send(Ok(response(2))).unwrap();
        });
        client.save(b"synthetic").await.unwrap();
        client.clear().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), peer)
            .await
            .expect("all expected requests must reach the synthetic peer")
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_request_permanently_invalidates_client() {
        let (mut client, mut receive) = fixture();
        {
            let request = client.load();
            tokio::pin!(request);
            tokio::select! {
                result = &mut request => panic!("unexpected result: {}", result.is_ok()),
                _ = receive.recv() => {}
            }
        }
        assert_eq!(client.load().await, Err(VaultError::Invalidated));
        assert_eq!(client.save(b"obsolete").await, Err(VaultError::Invalidated));
        assert!(receive.try_recv().is_err());
    }

    #[tokio::test]
    async fn write_failure_never_rebases_or_retries() {
        for error in [
            VaultError::Conflict,
            VaultError::Storage,
            VaultError::Transport,
        ] {
            let (mut client, mut receive) = fixture();
            client.cursor = Cursor::Loaded(Stamp {
                instance: [1; 32],
                revision: 0,
                hash: [2; 32],
            });
            let peer = tokio::spawn(async move {
                let (_, reply) = receive.recv().await.unwrap();
                reply.send(Err(error)).unwrap();
                receive
            });
            assert_eq!(client.save(b"canary-token").await, Err(error));
            assert_eq!(client.load().await, Err(VaultError::Invalidated));
            assert_eq!(client.clear().await, Err(VaultError::Invalidated));
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(1), peer)
                    .await
                    .expect("the failed write must reach the synthetic peer")
                    .unwrap()
                    .try_recv()
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn save_accepts_exact_limit_and_rejects_oversize_before_sending() {
        let (mut client, mut receive) = fixture();
        client.cursor = Cursor::Loaded(Stamp {
            instance: [1; 32],
            revision: 0,
            hash: [2; 32],
        });
        let peer = tokio::spawn(async move {
            let (request, reply) = receive.recv().await.unwrap();
            let Request::Write(_, Some(bytes)) = protocol::decode_request(&request).unwrap() else {
                panic!("save expected")
            };
            assert_eq!(bytes.len(), MAX_RECORD_BYTES);
            reply.send(Ok(response(1))).unwrap();
            receive
        });
        client.save(&vec![1; MAX_RECORD_BYTES]).await.unwrap();
        let mut receive = tokio::time::timeout(std::time::Duration::from_secs(1), peer)
            .await
            .expect("the maximum-sized request must reach the synthetic peer")
            .unwrap();
        let oversized = vec![1; MAX_RECORD_BYTES + 1];
        let result = tokio::select! {
            result = client.save(&oversized) => result,
            message = receive.recv() => panic!("oversized request reached transport: {}", message.is_some()),
        };
        assert_eq!(result, Err(VaultError::InvalidRequest));
        assert_eq!(client.load().await, Err(VaultError::Invalidated));
    }

    #[tokio::test]
    async fn malformed_write_acknowledgements_invalidate_the_client() {
        let expected = Stamp {
            instance: [1; 32],
            revision: 4,
            hash: [2; 32],
        };
        let updated = Stamp {
            revision: 5,
            ..expected
        };
        for response in [
            Response {
                stamp: Stamp {
                    instance: [9; 32],
                    ..updated
                },
                bytes: None,
            },
            Response {
                stamp: expected,
                bytes: None,
            },
            Response {
                stamp: updated,
                bytes: Some(Zeroizing::new(vec![0])),
            },
        ] {
            let (mut client, mut receive) = fixture();
            client.cursor = Cursor::Loaded(expected);
            let peer = tokio::spawn(async move {
                let (_, reply) = receive.recv().await.unwrap();
                reply
                    .send(Ok(protocol::encode_response(Ok(response))))
                    .unwrap();
                receive
            });
            assert_eq!(
                client.save(b"synthetic").await,
                Err(VaultError::InvalidRequest)
            );
            assert_eq!(client.load().await, Err(VaultError::Invalidated));
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(1), peer)
                    .await
                    .expect("the malformed acknowledgement must come from the synthetic peer")
                    .unwrap()
                    .try_recv()
                    .is_err()
            );
        }
    }
}
