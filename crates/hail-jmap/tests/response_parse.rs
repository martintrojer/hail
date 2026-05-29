use hail_jmap::jmap_client::core::error::MethodErrorType;
use hail_jmap::jmap_client::core::response::{EmailGetResponse, Response, SingleMethodResponse};

#[test]
fn email_get_request_too_large_method_error_deserializes() {
    let payload = serde_json::json!({
        "sessionState": "state-1",
        "methodResponses": [[
            "error",
            {
                "type": "requestTooLarge",
                "description": "Email/get with fetched bodyValues exceeded the server request limit"
            },
            "s0"
        ]]
    });

    let response: Response<SingleMethodResponse<EmailGetResponse>> =
        serde_json::from_value(payload).expect("requestTooLarge method response should parse");
    let method_response = response
        .unwrap_method_responses()
        .pop()
        .expect("method response");

    match method_response {
        SingleMethodResponse::Error((_, method_error, call_id)) => {
            assert_eq!(method_error.p_type, MethodErrorType::RequestTooLarge);
            assert_eq!(call_id, "s0");
        }
        SingleMethodResponse::Ok(_) => panic!("expected JMAP method error"),
    }
}
