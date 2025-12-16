//! Integration tests for IPC protocol serialization.

use rpytest_core::protocol::{ErrorCode, Outcome, Request, Response, TestEvent};
use rpytest_ipc::framing;

#[test]
fn request_encode_decode_roundtrip() {
    let requests = vec![
        Request::InitContext {
            repo_path: "/path/to/repo".to_string(),
            python_path: Some("/usr/bin/python3".to_string()),
        },
        Request::InitContext {
            repo_path: "/another/repo".to_string(),
            python_path: None,
        },
        Request::Collect {
            context_id: "ctx-123".to_string(),
            force: true,
        },
        Request::Collect {
            context_id: "ctx-456".to_string(),
            force: false,
        },
        Request::Run {
            context_id: "ctx-123".to_string(),
            node_ids: vec![
                "test_foo.py::test_bar".to_string(),
                "test_foo.py::TestClass::test_method".to_string(),
            ],
            workers: Some(4),
            maxfail: Some(1),
        },
        Request::Run {
            context_id: "ctx-123".to_string(),
            node_ids: vec![],
            workers: None,
            maxfail: None,
        },
        Request::List {
            context_id: "ctx-123".to_string(),
            keyword: Some("auth".to_string()),
            marker: Some("not slow".to_string()),
        },
        Request::List {
            context_id: "ctx-123".to_string(),
            keyword: None,
            marker: None,
        },
        Request::Shutdown {
            context_id: Some("ctx-123".to_string()),
        },
        Request::Shutdown { context_id: None },
        Request::Ping,
    ];

    for request in requests {
        // Encode with frame
        let frame = framing::encode(&request).unwrap();

        // Parse header
        let (len, offset) = framing::parse_frame_header(&frame).unwrap();
        assert_eq!(offset, 4);
        assert_eq!(len, frame.len() - 4);

        // Decode payload
        let decoded: Request = framing::decode(&frame[offset..]).unwrap();
        assert_eq!(request, decoded);

        // Also test encode_payload (without length prefix)
        let payload = framing::encode_payload(&request).unwrap();
        let decoded2: Request = framing::decode(&payload).unwrap();
        assert_eq!(request, decoded2);
    }
}

#[test]
fn response_encode_decode_roundtrip() {
    let responses = vec![
        Response::ContextReady {
            context_id: "ctx-123".to_string(),
            inventory_hash: "abc123def456".to_string(),
        },
        Response::CollectionComplete {
            node_count: 42,
            duration_ms: 150,
        },
        Response::TestList {
            node_ids: vec![
                "test_a.py::test_1".to_string(),
                "test_b.py::test_2".to_string(),
            ],
        },
        Response::TestList { node_ids: vec![] },
        Response::RunComplete {
            total: 100,
            passed: 95,
            failed: 3,
            skipped: 2,
            errors: 0,
            duration_ms: 5000,
        },
        Response::ShutdownAck,
        Response::Pong,
        Response::Error {
            code: ErrorCode::ContextNotFound,
            message: "Context 'ctx-999' not found".to_string(),
        },
        Response::Error {
            code: ErrorCode::CollectionFailed,
            message: "SyntaxError in test_foo.py".to_string(),
        },
        Response::Error {
            code: ErrorCode::InvalidRequest,
            message: "Missing required field".to_string(),
        },
        Response::Error {
            code: ErrorCode::InternalError,
            message: "Unexpected error".to_string(),
        },
        Response::Error {
            code: ErrorCode::Timeout,
            message: "Operation timed out".to_string(),
        },
        Response::Error {
            code: ErrorCode::PythonNotFound,
            message: "Python interpreter not found".to_string(),
        },
    ];

    for response in responses {
        let payload = framing::encode_payload(&response).unwrap();
        let decoded: Response = framing::decode(&payload).unwrap();
        assert_eq!(response, decoded);
    }
}

#[test]
fn test_event_encode_decode_roundtrip() {
    let events = vec![
        TestEvent::TestStart {
            node_id: "test_foo.py::test_bar".to_string(),
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_bar".to_string(),
            outcome: Outcome::Passed,
            duration_ms: 42,
            stdout: Some("test output".to_string()),
            stderr: None,
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_fail".to_string(),
            outcome: Outcome::Failed {
                message: "AssertionError: expected 1, got 2".to_string(),
            },
            duration_ms: 100,
            stdout: None,
            stderr: Some("error output".to_string()),
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_skip".to_string(),
            outcome: Outcome::Skipped {
                reason: Some("Requires database".to_string()),
            },
            duration_ms: 1,
            stdout: None,
            stderr: None,
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_error".to_string(),
            outcome: Outcome::Error {
                message: "ImportError: No module named 'missing'".to_string(),
            },
            duration_ms: 50,
            stdout: None,
            stderr: None,
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_xfail".to_string(),
            outcome: Outcome::XFail {
                reason: Some("Known bug #123".to_string()),
            },
            duration_ms: 30,
            stdout: None,
            stderr: None,
        },
        TestEvent::TestFinish {
            node_id: "test_foo.py::test_xpass".to_string(),
            outcome: Outcome::XPass,
            duration_ms: 20,
            stdout: None,
            stderr: None,
        },
        TestEvent::CollectionStart,
        TestEvent::ItemCollected {
            node_id: "test_foo.py::test_bar".to_string(),
            file_path: "test_foo.py".to_string(),
            line_number: Some(10),
            markers: vec!["slow".to_string(), "integration".to_string()],
        },
        TestEvent::CollectionFinish { count: 100 },
        TestEvent::SessionStart { test_count: 50 },
        TestEvent::SessionFinish { exit_code: 0 },
        TestEvent::SessionFinish { exit_code: 1 },
        TestEvent::Warning {
            message: "DeprecationWarning: This API is deprecated".to_string(),
            location: Some("test_foo.py:20".to_string()),
        },
        TestEvent::Warning {
            message: "UserWarning: Something happened".to_string(),
            location: None,
        },
    ];

    for event in events {
        let payload = framing::encode_payload(&event).unwrap();
        let decoded: TestEvent = framing::decode(&payload).unwrap();
        assert_eq!(event, decoded);
    }
}

#[test]
fn outcome_classification() {
    // Success outcomes
    assert!(Outcome::Passed.is_success());
    assert!(Outcome::Skipped { reason: None }.is_success());
    assert!(Outcome::Skipped {
        reason: Some("reason".to_string())
    }
    .is_success());
    assert!(Outcome::XFail { reason: None }.is_success());

    // Failure outcomes
    assert!(Outcome::Failed {
        message: "".to_string()
    }
    .is_failure());
    assert!(Outcome::XPass.is_failure());

    // Error outcomes
    assert!(Outcome::Error {
        message: "".to_string()
    }
    .is_error());

    // Not success, not failure, not error
    assert!(!Outcome::Passed.is_failure());
    assert!(!Outcome::Passed.is_error());
    assert!(!Outcome::Failed {
        message: "".to_string()
    }
    .is_success());
    assert!(!Outcome::Error {
        message: "".to_string()
    }
    .is_success());
}

#[test]
fn framing_rejects_oversized_messages() {
    // Create a message that's too large
    let large_data = vec![0u8; framing::MAX_MESSAGE_SIZE + 1];

    let result = framing::encode_payload(&large_data);
    assert!(result.is_err());
}

#[test]
fn framing_rejects_short_header() {
    let short_data = vec![0u8, 1u8, 2u8]; // Only 3 bytes, need 4 for header
    let result = framing::parse_frame_header(&short_data);
    assert!(result.is_err());
}

#[test]
fn framing_handles_empty_payload() {
    let empty: Vec<String> = vec![];
    let payload = framing::encode_payload(&empty).unwrap();
    let decoded: Vec<String> = framing::decode(&payload).unwrap();
    assert_eq!(empty, decoded);
}

#[test]
fn framing_handles_unicode() {
    let request = Request::InitContext {
        repo_path: "/путь/к/репо".to_string(), // Russian
        python_path: Some("/パイソン".to_string()), // Japanese
    };

    let payload = framing::encode_payload(&request).unwrap();
    let decoded: Request = framing::decode(&payload).unwrap();
    assert_eq!(request, decoded);
}

#[test]
fn framing_handles_special_characters() {
    let request = Request::List {
        context_id: "ctx-with-\"quotes\"-and-'apostrophes'".to_string(),
        keyword: Some("test\nwith\nnewlines".to_string()),
        marker: Some("marker\twith\ttabs".to_string()),
    };

    let payload = framing::encode_payload(&request).unwrap();
    let decoded: Request = framing::decode(&payload).unwrap();
    assert_eq!(request, decoded);
}
