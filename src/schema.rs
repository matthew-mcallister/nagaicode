// @generated automatically by Diesel CLI.

diesel::table! {
    item (id) {
        id -> Integer,
        session_id -> Integer,
        turn_id -> Integer,
        response_id -> Nullable<Integer>,
        provider_id -> Nullable<Integer>,
        #[sql_name = "type"]
        ty -> Text,
        upstream_id -> Nullable<Text>,
        upstream_type -> Nullable<Text>,
        upstream_call_id -> Nullable<Text>,
        text -> Nullable<Text>,
        summary -> Nullable<Text>,
        encrypted_text -> Nullable<Text>,
        json -> Nullable<Text>,
        raw_data -> Nullable<Text>,
        seqno -> BigInt,
        completed -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    model (provider_id, id) {
        provider_id -> Integer,
        id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    provider (id) {
        id -> Integer,
        name -> Text,
        interface -> Text,
        api_key -> Text,
        base_url -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    response (id) {
        id -> Integer,
        session_id -> Integer,
        turn_id -> Integer,
        upstream_id -> Nullable<Text>,
        upstream_status -> Nullable<Text>,
        input_tokens -> Nullable<BigInt>,
        cached_input_tokens -> Nullable<BigInt>,
        output_tokens -> Nullable<BigInt>,
        reasoning_tokens -> Nullable<BigInt>,
        total_tokens -> Nullable<BigInt>,
        raw_request -> Nullable<Text>,
        raw_response -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    setting (key) {
        key -> Text,
        value -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    session (id) {
        id -> Integer,
        name -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    turn (id) {
        id -> Integer,
        #[sql_name = "type"]
        ty -> Text,
        session_id -> Integer,
        provider_id -> Nullable<Integer>,
        provider_name -> Nullable<Text>,
        model_id -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(item -> response (response_id));
diesel::joinable!(item -> session (session_id));
diesel::joinable!(item -> turn (turn_id));
diesel::joinable!(model -> provider (provider_id));
diesel::joinable!(response -> session (session_id));
diesel::joinable!(response -> turn (turn_id));
diesel::joinable!(turn -> session (session_id));

diesel::allow_tables_to_appear_in_same_query!(
    item,
    model,
    provider,
    response,
    session,
    setting,
    turn,
);
