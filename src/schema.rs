// @generated automatically by Diesel CLI.

diesel::table! {
    chain (id) {
        id -> Integer,
        session_id -> Integer,
        provider_id -> Integer,
        provider_name -> Text,
        model_id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    content (id) {
        id -> Integer,
        item_id -> Integer,
        r#type -> Text,
        value -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    item (id) {
        id -> Integer,
        session_id -> Integer,
        chain_id -> Nullable<Integer>,
        r#type -> Text,
        response_id -> Nullable<Text>,
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
    session (id) {
        id -> Integer,
        name -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(chain -> session (session_id));
diesel::joinable!(content -> item (item_id));
diesel::joinable!(item -> chain (chain_id));
diesel::joinable!(item -> session (session_id));
diesel::joinable!(model -> provider (provider_id));

diesel::allow_tables_to_appear_in_same_query!(
    chain,
    content,
    item,
    model,
    provider,
    session,
);
