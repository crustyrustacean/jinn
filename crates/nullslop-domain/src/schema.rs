// @generated automatically by Diesel CLI.

diesel::table! {
    entries (id) {
        id -> Nullable<Text>,
        timestamp -> Text,
        kind -> Text,
    }
}

diesel::table! {
    session_entries (session_id, entry_id) {
        session_id -> Text,
        entry_id -> Text,
        ordinal -> Integer,
        pin_position -> Nullable<Text>,
        ignored -> Bool,
    }
}

diesel::table! {
    sessions (id) {
        id -> Nullable<Text>,
        title -> Nullable<Text>,
        updated_at -> Text,
        profile -> Text,
        strategy_state -> Text,
        blobs -> Text,
        parent_session -> Nullable<Text>,
        cwd -> Text,
        created_at -> Text,
        archived -> Bool,
    }
}

diesel::table! {
    token_ledger (id) {
        id -> Nullable<Integer>,
        session_id -> Text,
        timestamp -> Text,
        tokens_sent -> Integer,
        tokens_received -> Integer,
        cost -> Nullable<Double>,
    }
}

diesel::joinable!(session_entries -> entries (entry_id));
diesel::joinable!(session_entries -> sessions (session_id));
diesel::joinable!(token_ledger -> sessions (session_id));

diesel::allow_tables_to_appear_in_same_query!(entries, session_entries, sessions, token_ledger,);
