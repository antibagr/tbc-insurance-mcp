use crate::{MAX_RECORD_BYTES, VaultError, state::Stamp};
use zeroize::Zeroizing;

// One version byte, one operation/status byte, one fixed stamp, and one presence byte.
const STAMP_BYTES: usize = 72;
pub const MAX_FRAME_BYTES: usize = MAX_RECORD_BYTES + STAMP_BYTES + 3;
const VERSION: u8 = 1;

#[derive(Clone, Copy)]
pub enum Request<'a> {
    Load,
    Write(Stamp, Option<&'a [u8]>),
}

pub struct Response {
    pub(super) stamp: Stamp,
    pub(super) bytes: Option<Zeroizing<Vec<u8>>>,
}

pub fn encode_request(request: Request<'_>) -> Zeroizing<Vec<u8>> {
    let mut frame = Zeroizing::new(vec![VERSION]);
    match request {
        Request::Load => frame.push(0),
        Request::Write(stamp, bytes) => {
            frame.push(if bytes.is_some() { 1 } else { 2 });
            put_stamp(&mut frame, stamp);
            if let Some(bytes) = bytes {
                frame.extend_from_slice(bytes);
            }
        }
    }
    frame
}

pub fn decode_request(frame: &[u8]) -> Result<Request<'_>, VaultError> {
    if frame.len() > MAX_FRAME_BYTES || frame.first() != Some(&VERSION) {
        return Err(VaultError::InvalidRequest);
    }
    match frame.get(1) {
        Some(0) if frame.len() == 2 => Ok(Request::Load),
        Some(1 | 2) if frame.len() >= STAMP_BYTES + 2 => {
            let stamp = get_stamp(&frame[2..STAMP_BYTES + 2])?;
            let bytes = &frame[STAMP_BYTES + 2..];
            match frame[1] {
                1 if bytes.len() <= MAX_RECORD_BYTES => Ok(Request::Write(stamp, Some(bytes))),
                2 if bytes.is_empty() => Ok(Request::Write(stamp, None)),
                _ => Err(VaultError::InvalidRequest),
            }
        }
        _ => Err(VaultError::InvalidRequest),
    }
}

pub fn encode_response(result: Result<Response, VaultError>) -> Zeroizing<Vec<u8>> {
    let mut frame = Zeroizing::new(vec![VERSION]);
    match result {
        Ok(response) => {
            frame.push(0);
            put_stamp(&mut frame, response.stamp);
            frame.push(u8::from(response.bytes.is_some()));
            if let Some(bytes) = response.bytes {
                frame.extend_from_slice(&bytes);
            }
        }
        Err(error) => frame.push(match error {
            VaultError::Conflict => 1,
            VaultError::InvalidRequest => 2,
            _ => 3,
        }),
    }
    frame
}

pub fn decode_response(frame: &[u8]) -> Result<Response, VaultError> {
    if frame.len() > MAX_FRAME_BYTES || frame.first() != Some(&VERSION) {
        return Err(VaultError::InvalidRequest);
    }
    if frame.len() == 2 {
        return Err(match frame[1] {
            1 => VaultError::Conflict,
            3 => VaultError::Storage,
            _ => VaultError::InvalidRequest,
        });
    }
    if frame.get(1) != Some(&0) || frame.len() < STAMP_BYTES + 3 {
        return Err(VaultError::InvalidRequest);
    }
    let stamp = get_stamp(&frame[2..STAMP_BYTES + 2])?;
    let bytes = &frame[STAMP_BYTES + 3..];
    let bytes = match frame[STAMP_BYTES + 2] {
        0 if bytes.is_empty() => None,
        1 => Some(Zeroizing::new(bytes.to_vec())),
        _ => return Err(VaultError::InvalidRequest),
    };
    Ok(Response { stamp, bytes })
}

fn put_stamp(frame: &mut Vec<u8>, stamp: Stamp) {
    frame.extend_from_slice(&stamp.instance);
    frame.extend_from_slice(&stamp.revision.to_be_bytes());
    frame.extend_from_slice(&stamp.hash);
}

fn get_stamp(bytes: &[u8]) -> Result<Stamp, VaultError> {
    if bytes.len() != STAMP_BYTES {
        return Err(VaultError::InvalidRequest);
    }
    Ok(Stamp {
        instance: bytes[..32]
            .try_into()
            .map_err(|_| VaultError::InvalidRequest)?,
        revision: u64::from_be_bytes(
            bytes[32..40]
                .try_into()
                .map_err(|_| VaultError::InvalidRequest)?,
        ),
        hash: bytes[40..]
            .try_into()
            .map_err(|_| VaultError::InvalidRequest)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Stamp {
        Stamp {
            instance: [3; 32],
            revision: 42,
            hash: [9; 32],
        }
    }

    #[test]
    fn binary_frames_round_trip_and_enforce_exact_boundaries() {
        for data in [vec![], vec![255; MAX_RECORD_BYTES]] {
            let frame = encode_request(Request::Write(stamp(), Some(&data)));
            let Request::Write(actual, Some(bytes)) = decode_request(&frame).unwrap() else {
                panic!("write expected")
            };
            assert_eq!(actual, stamp());
            assert_eq!(bytes, data);
            let reply = encode_response(Ok(Response {
                stamp: stamp(),
                bytes: Some(Zeroizing::new(data.clone())),
            }));
            assert!(reply.len() <= MAX_FRAME_BYTES);
            assert_eq!(
                decode_response(&reply).unwrap().bytes.unwrap().as_slice(),
                data
            );
        }
        assert!(matches!(
            decode_request(&encode_request(Request::Load)),
            Ok(Request::Load)
        ));
        assert!(matches!(
            decode_request(&encode_request(Request::Write(stamp(), None))),
            Ok(Request::Write(_, None))
        ));
        let too_large = encode_request(Request::Write(
            stamp(),
            Some(&vec![0; MAX_RECORD_BYTES + 1]),
        ));
        assert!(decode_request(&too_large).is_err());
        for bytes in [
            vec![],
            vec![2, 0],
            vec![1, 0, 0],
            vec![1, 3],
            vec![1; MAX_FRAME_BYTES + 1],
        ] {
            assert!(decode_request(&bytes).is_err());
            assert!(decode_response(&bytes).is_err());
        }
    }

    #[test]
    fn no_record_differs_from_empty_record_and_errors_are_closed() {
        let frame = encode_response(Ok(Response {
            stamp: stamp(),
            bytes: None,
        }));
        assert!(decode_response(&frame).unwrap().bytes.is_none());
        for error in [
            VaultError::Conflict,
            VaultError::Storage,
            VaultError::InvalidRequest,
        ] {
            assert!(
                matches!(decode_response(&encode_response(Err(error))), Err(actual) if actual == error)
            );
        }
        let mut absent_with_payload = frame.to_vec();
        absent_with_payload.push(1);
        assert!(decode_response(&absent_with_payload).is_err());
        let mut clear_with_payload = encode_request(Request::Write(stamp(), None));
        clear_with_payload.push(1);
        assert!(decode_request(&clear_with_payload).is_err());
    }

    #[test]
    fn every_truncated_fixed_header_is_rejected_without_panicking() {
        let request = encode_request(Request::Write(stamp(), Some(&[])));
        for length in 0..request.len() {
            assert!(
                decode_request(&request[..length]).is_err(),
                "request prefix {length}"
            );
        }
        let response = encode_response(Ok(Response {
            stamp: stamp(),
            bytes: None,
        }));
        for length in 0..response.len() {
            assert!(
                decode_response(&response[..length]).is_err(),
                "response prefix {length}"
            );
        }
    }

    #[test]
    fn response_budget_version_and_status_are_independent_boundaries() {
        let oversized = encode_response(Ok(Response {
            stamp: stamp(),
            bytes: Some(Zeroizing::new(vec![0; MAX_RECORD_BYTES + 1])),
        }));
        assert!(decode_response(&oversized).is_err());
        let mut wrong_version = encode_response(Ok(Response {
            stamp: stamp(),
            bytes: None,
        }));
        wrong_version[0] = VERSION + 1;
        assert!(decode_response(&wrong_version).is_err());
        let mut wrong_status = encode_response(Ok(Response {
            stamp: stamp(),
            bytes: None,
        }));
        wrong_status[1] = 7;
        assert!(decode_response(&wrong_status).is_err());
    }
}
