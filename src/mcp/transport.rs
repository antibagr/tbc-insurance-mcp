//! Size-bounded newline-delimited stdio transport.

use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::{SinkExt, StreamExt};
use rmcp::{
    RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::JsonRpcMessageCodec},
};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
};
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, Encoder, FramedRead, FramedWrite},
};

/// Maximum JSON bytes in one inbound or outbound MCP stdio frame.
pub const MAX_MCP_FRAME_BYTES: usize = 1024 * 1024;

/// Create the production MCP transport over standard input and output.
#[must_use]
pub fn bounded_stdio() -> (
    impl Transport<RoleServer, Error = rmcp::transport::async_rw::JsonRpcMessageCodecError> + 'static,
    StdioInputStatus,
) {
    BoundedStdioTransport::with_status(tokio::io::stdin(), tokio::io::stdout(), MAX_MCP_FRAME_BYTES)
}

/// Reports whether the stdio reader rejected an input frame.
#[derive(Clone, Default)]
pub struct StdioInputStatus(Arc<AtomicBool>);

impl StdioInputStatus {
    /// Return `true` after a read or frame-decoding failure.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn mark_failed(&self) {
        self.0.store(true, Ordering::Release);
    }
}

type TransportError = rmcp::transport::async_rw::JsonRpcMessageCodecError;
type Reader<R> = FramedRead<R, StrictInboundCodec<RxJsonRpcMessage<RoleServer>>>;
type Writer<W> = FramedWrite<W, BoundedJsonRpcEncoder<TxJsonRpcMessage<RoleServer>>>;

struct BoundedStdioTransport<R, W> {
    reader: Reader<R>,
    writer: Arc<Mutex<Option<Writer<W>>>>,
    input_status: StdioInputStatus,
}

impl<R, W> BoundedStdioTransport<R, W>
where
    R: AsyncRead,
    W: AsyncWrite,
{
    fn with_status(reader: R, writer: W, max_frame_bytes: usize) -> (Self, StdioInputStatus) {
        let input_status = StdioInputStatus::default();
        (
            Self::with_input_status(reader, writer, max_frame_bytes, input_status.clone()),
            input_status,
        )
    }

    fn with_input_status(
        reader: R,
        writer: W,
        max_frame_bytes: usize,
        input_status: StdioInputStatus,
    ) -> Self {
        Self {
            reader: FramedRead::new(reader, StrictInboundCodec::new(max_frame_bytes)),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                writer,
                BoundedJsonRpcEncoder::new(max_frame_bytes),
            )))),
            input_status,
        }
    }
}

impl<R, W> Transport<RoleServer> for BoundedStdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = TransportError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = Arc::clone(&self.writer);
        async move {
            writer
                .lock()
                .await
                .as_mut()
                .ok_or_else(disconnected_error)?
                .send(item)
                .await
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        match self.reader.next().await {
            Some(Ok(message)) => Some(message),
            Some(Err(_)) => {
                self.input_status.mark_failed();
                None
            }
            None => None,
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let writer = self.writer.lock().await.take();
        if let Some(mut writer) = writer {
            writer.close().await?;
        }
        Ok(())
    }
}

fn disconnected_error() -> TransportError {
    io::Error::new(io::ErrorKind::NotConnected, "MCP stdio transport is closed").into()
}

struct StrictInboundCodec<T>(JsonRpcMessageCodec<T>);

impl<T> StrictInboundCodec<T> {
    fn new(max_length: usize) -> Self {
        Self(JsonRpcMessageCodec::new_with_max_length(max_length))
    }
}

impl<T: serde::de::DeserializeOwned> Decoder for StrictInboundCodec<T> {
    type Item = T;
    type Error = TransportError;

    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.0.decode(source)
    }

    fn decode_eof(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(item) = self.decode(source)? {
            return Ok(Some(item));
        }
        if source.is_empty() {
            Ok(None)
        } else {
            source.clear();
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MCP stdio frame is missing its newline delimiter",
            )
            .into())
        }
    }
}

struct BoundedJsonRpcEncoder<T> {
    inner: JsonRpcMessageCodec<T>,
    max_length: usize,
}

impl<T> BoundedJsonRpcEncoder<T> {
    fn new(max_length: usize) -> Self {
        Self {
            inner: JsonRpcMessageCodec::new_with_max_length(max_length),
            max_length,
        }
    }
}

impl<T: Serialize> Encoder<T> for BoundedJsonRpcEncoder<T> {
    type Error = TransportError;

    fn encode(&mut self, item: T, destination: &mut BytesMut) -> Result<(), Self::Error> {
        let mut counter = ByteCounter::default();
        serde_json::to_writer(&mut counter, &item)?;
        if counter.bytes > self.max_length {
            return Err(TransportError::MaxLineLengthExceeded);
        }
        self.inner.encode(item, destination)
    }
}

#[derive(Default)]
struct ByteCounter {
    bytes: usize,
}

impl Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_limit_accepts_exact_size_and_rejects_one_byte_over() {
        let message = serde_json::json!({"value": "covered"});
        let length = serde_json::to_vec(&message).expect("serializable").len();
        let mut destination = BytesMut::new();
        BoundedJsonRpcEncoder::new(length)
            .encode(message.clone(), &mut destination)
            .expect("exact-size frame");
        assert_eq!(destination.len(), length + 1);

        let error = BoundedJsonRpcEncoder::new(length - 1)
            .encode(message, &mut BytesMut::new())
            .expect_err("oversized frame");
        assert!(matches!(error, TransportError::MaxLineLengthExceeded));
    }

    #[test]
    fn inbound_limit_accepts_exact_size_and_rejects_one_byte_over() {
        let message = serde_json::json!({"value": "covered"});
        let mut encoded = serde_json::to_vec(&message).expect("serializable");
        let length = encoded.len();
        encoded.push(b'\n');

        let mut exact = BytesMut::from(encoded.as_slice());
        let decoded = StrictInboundCodec::<serde_json::Value>::new(length)
            .decode(&mut exact)
            .expect("exact-size frame")
            .expect("decoded frame");
        assert_eq!(decoded, message);

        let mut oversized = BytesMut::from(encoded.as_slice());
        let error = StrictInboundCodec::<serde_json::Value>::new(length - 1)
            .decode(&mut oversized)
            .expect_err("oversized frame");
        assert!(matches!(error, TransportError::MaxLineLengthExceeded));
    }

    #[test]
    fn inbound_frame_requires_a_newline_before_eof() {
        let mut unterminated = BytesMut::from(&b"{\"jsonrpc\":\"2.0\"}"[..]);
        let error = StrictInboundCodec::<serde_json::Value>::new(MAX_MCP_FRAME_BYTES)
            .decode_eof(&mut unterminated)
            .expect_err("unterminated frame");
        assert!(matches!(error, TransportError::Io(_)));
    }

    #[tokio::test]
    async fn closing_transport_removes_its_writer() {
        let (mut transport, _) = BoundedStdioTransport::with_status(
            tokio::io::empty(),
            tokio::io::sink(),
            MAX_MCP_FRAME_BYTES,
        );
        transport.close().await.expect("close transport");
        assert!(transport.writer.lock().await.is_none());
    }
}
