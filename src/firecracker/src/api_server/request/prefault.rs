// Copyright 2026 kvcache-ai. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use vmm::rpc_interface::VmmAction;
use vmm::vstate::prefault::PreFaultMemoryRequest;

use super::super::parsed_request::{ParsedRequest, RequestError};
use super::Body;

/// Parses a request to pre-fault selected guest memory.
pub(crate) fn parse_put_pre_fault_memory(body: &Body) -> Result<ParsedRequest, RequestError> {
    let request = serde_json::from_slice::<PreFaultMemoryRequest>(body.raw())?;
    request.validate().map_err(|error| {
        RequestError::Generic(micro_http::StatusCode::BadRequest, error.to_string())
    })?;
    Ok(ParsedRequest::new_sync(VmmAction::PreFaultMemory(request)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_server::parsed_request::tests::vmm_action_from_request;
    use vmm::vstate::prefault::PreFaultMemoryRange;

    #[test]
    fn test_parse_put_pre_fault_memory_request() {
        let parsed =
            parse_put_pre_fault_memory(&Body::new(r#"{"ranges":[{"gpa":4096,"size":16384}]}"#))
                .unwrap();
        assert_eq!(
            vmm_action_from_request(parsed),
            VmmAction::PreFaultMemory(PreFaultMemoryRequest {
                ranges: vec![PreFaultMemoryRange {
                    gpa: 4096,
                    size: 16384,
                }],
            })
        );
    }

    #[test]
    fn test_parse_put_pre_fault_memory_rejects_invalid_ranges() {
        for body in [
            r#"{"ranges":[]}"#,
            r#"{"ranges":[{"gpa":4096,"size":0}]}"#,
            r#"{"ranges":[{"gpa":1,"size":4096}]}"#,
            r#"{"ranges":[{"gpa":-1,"size":4096}]}"#,
            r#"{"ranges":[{"gpa":18446744073709551616,"size":4096}]}"#,
            r#"{"ranges":[{"gpa":18446744073709547520,"size":8192}]}"#,
            r#"{"ranges":[{"gpa":4096,"size":4096,"extra":true}]}"#,
            r#"{"ranges":[{"gpa":4096,"size":16384}],"extra":true}"#,
        ] {
            assert!(
                parse_put_pre_fault_memory(&Body::new(body)).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn test_parse_put_pre_fault_memory_preserves_duplicate_ranges() {
        let request = parse_put_pre_fault_memory(&Body::new(
            r#"{"ranges":[{"gpa":0,"size":4096},{"gpa":0,"size":4096}]}"#,
        ))
        .unwrap();
        assert_eq!(
            vmm_action_from_request(request),
            VmmAction::PreFaultMemory(PreFaultMemoryRequest {
                ranges: vec![
                    PreFaultMemoryRange { gpa: 0, size: 4096 },
                    PreFaultMemoryRange { gpa: 0, size: 4096 },
                ],
            })
        );
    }
}
