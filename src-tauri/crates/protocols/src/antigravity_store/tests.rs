use super::*;
use std::cell::Cell;

// Synthetic fields follow the installed 1.5.10 FileDescriptorProto metadata. No user store is read.
fn var(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n == 0 { 0 } else { 128 });
        if n == 0 {
            return out;
        }
    }
}
fn field(n: u32, bytes: &[u8]) -> Vec<u8> {
    [
        var((u64::from(n) << 3) | 2),
        var(bytes.len() as u64),
        bytes.into(),
    ]
    .concat()
}
fn int(n: u32, value: u64) -> Vec<u8> {
    [var(u64::from(n) << 3), var(value)].concat()
}
struct Bytes(Vec<u8>, Cell<usize>);
impl Source for Bytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
        self.1.set(self.1.get().max(bytes.len()));
        self.0.as_slice().read_at(offset, bytes)
    }
}
fn bytes(value: Vec<u8>) -> Bytes {
    Bytes(value, Cell::new(0))
}
fn call(id: &str) -> Vec<u8> {
    [
        field(1, id.as_bytes()),
        field(2, b"run_command"),
        field(3, br#"{"command":"printf MARKER"}"#),
    ]
    .concat()
}
fn payload(kind: u64, status: u64, metadata: &[u8], n: u32, body: &[u8]) -> Vec<u8> {
    [
        int(1, kind),
        int(4, status),
        field(5, metadata),
        field(n, body),
    ]
    .concat()
}
const SCHEMA: Schema = Schema::Cli1510;

#[test]
fn hidden_native_resume_message_is_receipted_without_public_content_or_completion() {
    let system = |render_hidden, agent_hidden| {
        [
            field(
                1,
                b"[Message] sender=system: Background tasks were stopped on resume.",
            ),
            field(
                2,
                &[
                    int(4, render_hidden),
                    field(10, &field(1, b"Internal resume bookkeeping.")),
                ]
                .concat(),
            ),
            field(
                4,
                &[
                    field(1, b"native-system-message"),
                    field(4, b"Internal resume bookkeeping."),
                    int(11, agent_hidden),
                ]
                .concat(),
            ),
        ]
        .concat()
    };
    let raw = bytes(payload(101, 3, &int(3, 5), 114, &system(1, 1)));
    let hidden = step(&raw, 3, 101, 3, SCHEMA).unwrap();
    assert_eq!(hidden.content, Content::HiddenSystemMessage);
    assert!(hidden.complete);
    let proof = projection::completion_record(&hidden);
    assert_eq!(proof["type"], "SYSTEM_MESSAGE");
    assert_eq!(proof["source"], "SYSTEM");
    assert!(proof["content"].is_null());
    let mut projection = projection::Projection::new("session", "trajectory").unwrap();
    let captures = projection.push(&hidden, 1).unwrap();
    assert_eq!(captures.len(), 1);
    assert!(captures[0].frame.is_none());
    assert_eq!(captures[0].checkpoint["sourceScope"], "record");
    assert_eq!(captures[0].checkpoint["kind"], "turn");
    for body in [
        system(0, 1),
        system(1, 0),
        [system(1, 1), field(6, b"media")].concat(),
        [field(1, b"Visible native notice"), field(2, &int(4, 1))].concat(),
    ] {
        assert_eq!(
            step(
                &bytes(payload(101, 3, &int(3, 5), 114, &body)),
                3,
                101,
                3,
                SCHEMA
            ),
            Err(Error::Unsupported)
        );
    }
    assert_eq!(
        step(
            &bytes(payload(101, 3, &int(3, 2), 114, &system(1, 1))),
            3,
            101,
            3,
            SCHEMA
        ),
        Err(Error::Identity)
    );
    let invalid_utf8 = [field(1, &[0xff]), field(2, &int(4, 1))].concat();
    assert_eq!(
        step(
            &bytes(payload(101, 3, &int(3, 5), 114, &invalid_utf8)),
            3,
            101,
            3,
            SCHEMA
        ),
        Err(Error::Malformed)
    );
    let excessive = field(1, &vec![b'x'; TEXT_LIMIT + 1]);
    assert_eq!(
        step(
            &bytes(payload(101, 3, &int(3, 5), 114, &excessive)),
            3,
            101,
            3,
            SCHEMA
        ),
        Err(Error::Limit)
    );
    let mut truncated = payload(101, 3, &int(3, 5), 114, &system(1, 1));
    truncated.pop();
    assert_eq!(
        step(&bytes(truncated), 3, 101, 3, SCHEMA),
        Err(Error::Malformed)
    );
}

#[test]
fn native_checkpoint_decodes_system_summary_without_a_model_response() {
    let body = [field(5, b"Summary"), field(6, b"Changed file"), int(17, 1)].concat();
    let raw = bytes(payload(23, 3, &int(3, 5), 30, &body));
    let checkpoint = step(&raw, 4, 23, 3, SCHEMA).unwrap();
    assert_eq!(
        checkpoint.content,
        Content::Checkpoint {
            summary: "Summary\n\nChanged file".into(),
            failed: false
        }
    );
    assert!(checkpoint.complete);
    assert_eq!(
        projection::completion_record(&checkpoint)["type"],
        "CHECKPOINT"
    );
    let wrong_source = bytes(payload(23, 3, &int(3, 2), 30, &body));
    assert_eq!(step(&wrong_source, 4, 23, 3, SCHEMA), Err(Error::Identity));
    let unresolved = bytes(payload(
        23,
        3,
        &int(3, 5),
        30,
        &field(29, b"unread-summary"),
    ));
    assert_eq!(step(&unresolved, 4, 23, 3, SCHEMA), Err(Error::Unsupported));
    let failed = bytes(payload(23, 3, &int(3, 5), 30, &int(16, 1)));
    assert!(matches!(
        step(&failed, 4, 23, 3, SCHEMA).unwrap().content,
        Content::Checkpoint { failed: true, .. }
    ));
}

#[test]
fn native_tool_result_retains_explicit_call_identity_without_adjacency() {
    let first = call("call-one");
    let second = call("call-two");
    let planner = bytes(payload(
        15,
        3,
        &int(3, 2),
        20,
        &[field(1, b"Answer"), field(7, &first), field(7, &second)].concat(),
    ));
    let answer = step(&planner, 4, 15, 3, SCHEMA).unwrap();
    let Content::Assistant { calls, .. } = answer.content else {
        panic!()
    };
    assert_eq!(calls.len(), 2);
    let result = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &second)].concat(),
        140,
        &field(2, &field(1, b"MARKER")),
    ));
    let done = step(&result, 18, 132, 3, SCHEMA).unwrap();
    let Content::Tool { call, result, .. } = done.content else {
        panic!()
    };
    assert_eq!(call, calls[1]);
    assert_eq!(result, Some(json!("MARKER")));
    assert!(done.complete);
}
#[test]
fn repeated_text_distinct_calls_and_mutable_result_versions_remain_distinct() {
    let make = |id: &str, status, output: &[u8]| {
        bytes(payload(
            132,
            status,
            &[int(3, 2), field(4, &call(id))].concat(),
            140,
            &field(2, &field(1, output)),
        ))
    };
    let first = step(&make("first", 2, b"Part"), 3, 132, 2, SCHEMA).unwrap();
    let final_step = step(&make("first", 3, b"Final"), 3, 132, 3, SCHEMA).unwrap();
    let repeated = step(&make("second", 3, b"Final"), 5, 132, 3, SCHEMA).unwrap();
    assert_ne!(first.version, final_step.version);
    assert_ne!(final_step.version, repeated.version);
    assert_eq!(first.index, final_step.index);
    assert!(!first.status.terminal());
    assert!(final_step.status.terminal());
}
#[test]
fn required_text_is_bounded_while_large_binary_context_is_skipped_and_hashed() {
    let mut user = [field(2, b"Exact user text"), field(19, b"native-client")].concat();
    user.extend(field(9, &vec![7; 2 * TEXT_LIMIT]));
    let input = bytes(payload(14, 3, &int(3, 4), 19, &user));
    let decoded = step(&input, 0, 14, 3, SCHEMA).unwrap();
    assert_eq!(
        decoded.content,
        Content::User {
            text: "Exact user text".into(),
            client_id: Some("native-client".into())
        }
    );
    assert!(decoded.complete);
    assert!(input.1.get() <= CHUNK);
    let huge = bytes(payload(
        14,
        3,
        &int(3, 4),
        19,
        &field(2, &vec![b'x'; TEXT_LIMIT + 1]),
    ));
    assert_eq!(step(&huge, 0, 14, 3, SCHEMA), Err(Error::Limit));
}
#[test]
fn mismatched_headers_missing_identity_and_malformed_framing_fail_closed() {
    let good = payload(
        132,
        3,
        &[int(3, 2), field(4, &call("call"))].concat(),
        140,
        &field(2, &field(1, b"Output")),
    );
    assert_eq!(
        step(&bytes(good.clone()), 1, 132, 2, SCHEMA),
        Err(Error::Identity)
    );
    let missing = bytes(payload(
        132,
        3,
        &int(3, 2),
        140,
        &field(2, &field(1, b"Output")),
    ));
    assert_eq!(step(&missing, 1, 132, 3, SCHEMA), Err(Error::Identity));
    let mut malformed = good;
    malformed.extend([0x82, 0x01, 0xff, 0xff]);
    assert_eq!(
        step(&bytes(malformed), 1, 132, 3, SCHEMA),
        Err(Error::Malformed)
    );
}
#[test]
fn structured_results_decode_values_and_external_output_reference_withholds_coverage() {
    let entry = [field(1, b"ok"), field(2, &int(4, 1))].concat();
    let result = [
        field(10, &field(1, &entry)),
        field(7, b"file:///authorized-but-not-read/output"),
    ]
    .concat();
    let input = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &call("call"))].concat(),
        140,
        &field(2, &result),
    ));
    let step = step(&input, 2, 132, 3, SCHEMA).unwrap();
    assert!(!step.complete);
    assert!(
        matches!(step.content,Content::Tool{result:Some(value),..} if value==json!({"ok":true}))
    );
}

#[test]
fn model_usage_preserves_native_zero_defaults_and_cache_read_semantics() {
    let usage = [int(2, 40), int(3, 12), int(5, 60), int(9, 4)].concat();
    let metadata = [
        int(3, 2),
        field(9, &usage),
        field(24, &field(8, b"native-model")),
    ]
    .concat();
    let input = bytes(payload(15, 3, &metadata, 20, &field(1, b"Answer")));
    let step = step(&input, 1, 15, 3, SCHEMA).unwrap();
    let usage = step.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(100));
    assert_eq!(usage.output_tokens, Some(12));
    assert_eq!(usage.cache_read_tokens, Some(60));
    assert_eq!(usage.cache_write_tokens, Some(0));
    assert_eq!(usage.reasoning_tokens, Some(4));
    assert_eq!(step.model.as_deref(), Some("native-model"));
}
#[test]
fn nested_result_depth_and_terminal_missing_result_withhold_coverage() {
    let mut value = int(4, 1);
    for _ in 0..20 {
        value = field(6, &field(1, &value));
    }
    let structure = field(1, &[field(1, b"nested"), field(2, &value)].concat());
    let input = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &call("native-call"))].concat(),
        140,
        &field(2, &field(10, &structure)),
    ));
    assert_eq!(step(&input, 2, 132, 3, SCHEMA), Err(Error::Limit));
    let missing = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &call("native-call"))].concat(),
        140,
        &[],
    ));
    assert!(!step(&missing, 2, 132, 3, SCHEMA).unwrap().complete);
}

#[test]
fn edited_native_tool_arguments_require_the_same_call_identity() {
    let original = call("same-call");
    let edited = |id: &str| {
        [
            field(1, id.as_bytes()),
            field(2, b"run_command"),
            field(3, br#"{"command":"printf EDITED"}"#),
        ]
        .concat()
    };
    let make = |id: &str| {
        bytes(payload(
            132,
            3,
            &[int(3, 2), field(4, &original), field(29, &edited(id))].concat(),
            140,
            &field(2, &field(1, b"EDITED")),
        ))
    };
    let valid = step(&make("same-call"), 2, 132, 3, SCHEMA).unwrap();
    assert!(
        matches!(valid.content,Content::Tool{call,..}if call.input["command"]=="printf EDITED")
    );
    assert_eq!(
        step(&make("another-call"), 2, 132, 3, SCHEMA),
        Err(Error::Identity)
    );
}

#[test]
fn auxiliary_any_metadata_does_not_hide_complete_generic_result_or_native_error() {
    let read_call = [
        field(1, b"read-call"),
        field(2, b"read_file"),
        field(3, br#"{"path":"fixture.txt"}"#),
    ]
    .concat();
    let auxiliary = field(
        6,
        &[
            field(1, b"type.googleapis.com/exa.cortex.core.tools.ReadFile"),
            field(2, &[int(1, 3), field(3, b"fixture.txt")].concat()),
        ]
        .concat(),
    );
    let make = |status, error: Option<&str>, output: bool| {
        let mut result = auxiliary.clone();
        if output {
            result.extend(field(1, b"Full native file content"));
        }
        let mut body = payload(
            132,
            status,
            &[int(3, 2), field(4, &read_call)].concat(),
            140,
            &field(2, &result),
        );
        if let Some(error) = error {
            body.extend(field(31, &field(1, error.as_bytes())));
        }
        bytes(body)
    };
    let success = step(&make(3, None, true), 2, 132, 3, SCHEMA).unwrap();
    assert!(success.complete);
    assert!(
        matches!(success.content,Content::Tool{error:false,result:Some(value),..}if value=="Full native file content")
    );
    let failure = step(&make(3, Some("Native file error"), true), 2, 132, 3, SCHEMA).unwrap();
    assert!(matches!(failure.content, Content::Tool { error: true, .. }));
    let invalid = step(&make(4, None, true), 2, 132, 4, SCHEMA).unwrap();
    assert!(invalid.status.terminal());
    assert!(matches!(invalid.content, Content::Tool { error: true, .. }));
    let error_output = step(
        &make(7, Some("Native file error"), false),
        2,
        132,
        7,
        SCHEMA,
    )
    .unwrap();
    assert!(error_output.complete);
    assert!(
        matches!(error_output.content,Content::Tool{error:true,result:Some(value),..}if value=="Native file error")
    );
    assert_eq!(
        step(&make(3, None, false), 2, 132, 3, SCHEMA),
        Err(Error::Unsupported)
    );
}
#[test]
fn supplementary_any_still_requires_valid_framing_and_run_command_outcome() {
    let native_call = [
        field(1, b"native-call"),
        field(2, b"other_tool"),
        field(3, b"{}"),
    ]
    .concat();
    let result = [
        field(1, b"Output"),
        field(
            6,
            &[
                field(1, b"type.googleapis.com/auxiliary.Unknown"),
                field(2, &[0xff]),
            ]
            .concat(),
        ),
    ]
    .concat();
    let input = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &native_call)].concat(),
        140,
        &field(2, &result),
    ));
    assert_eq!(step(&input, 2, 132, 3, SCHEMA), Err(Error::Malformed));
    let result = [
        field(1, b"Output"),
        field(
            6,
            &[
                field(1, b"type.googleapis.com/exa.cortex.core.tools.RunCommand"),
                field(2, &int(8, 1)),
            ]
            .concat(),
        ),
    ]
    .concat();
    let input = bytes(payload(
        132,
        3,
        &[int(3, 2), field(4, &call("native-call"))].concat(),
        140,
        &field(2, &result),
    ));
    assert_eq!(step(&input, 2, 132, 3, SCHEMA), Err(Error::Unsupported));
}
