// @generated automatically by Diesel CLI.
//
// NOTE: provider.id is manually corrected from Nullable<Integer> to Integer.
// SQLite's PRAGMA table_info reports INTEGER PRIMARY KEY as nullable, so Diesel
// CLI infers it as Nullable<Integer>. This is incorrect — the column is always
// non-null — so we patch it here to match the Provider struct's `id: i32`.

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

diesel::joinable!(model -> provider (provider_id));

diesel::allow_tables_to_appear_in_same_query!(
    model,
    provider,
);
