// @generated automatically by Diesel CLI.

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
